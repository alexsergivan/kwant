//! Work from Jira (jira-tickets spec J4, J5, J14, J15): one Jira site per space, the tickets Dev
//! mode picked up from it, and what happened to each. Add-only tables, left out of exports (an
//! imported bundle can't connect a site or start a ticket), written only by the person's routes and
//! the pickup poller. The token is never here: it's in the password vault under the site's id.
//!
//! `set_dev_ticket_state` is the one writer of `dev_tickets.state` (api.md §1.4): it refuses a
//! move the table below doesn't allow and logs every move it makes in `dev_ticket_events`.
//! `tickets::store::set_state` wraps it to send `dev_ticket` on the event stream. The ticket's brief
//! row (`dev_ticket_briefs.state`) is moved in the same transaction.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

pub const WAITING: &str = "waiting";
pub const PICKED_UP: &str = "picked_up";
pub const READING: &str = "reading";
pub const NEEDS_ANSWER: &str = "needs_answer";
pub const BRIEF_READY: &str = "brief_ready";
pub const WORKING: &str = "working";
pub const IN_REVIEW: &str = "in_review";
pub const LANDED: &str = "landed";
pub const TAKEN_BACK: &str = "taken_back";
pub const CLOSED: &str = "closed";
pub const FAILED: &str = "failed";
/// The `error_code` of a row failed by a writer that didn't name a reason.
pub const FAIL_DEFAULT_CODE: &str = "failed";

pub const STATES: &[&str] = &[WAITING, PICKED_UP, READING, NEEDS_ANSWER, BRIEF_READY, WORKING, IN_REVIEW, LANDED, TAKEN_BACK, CLOSED, FAILED];

/// States pickup keeps watching in Jira (it re-reads them each poll for J14).
pub const ACTIVE: &[&str] = &[WAITING, PICKED_UP, READING, NEEDS_ANSWER, BRIEF_READY, WORKING, IN_REVIEW, TAKEN_BACK, FAILED];

/// States that hold one of the 3 slots (J13). `needs_answer` waits on a person and doesn't.
pub const SLOT_STATES: &[&str] = &[READING, BRIEF_READY, WORKING];

/// The person's lifecycle buttons on a ticket card (api-decisions 12, api.md §1.4). None of them
/// writes to Jira.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonMove {
    /// Stop the work, keep the branch → `taken_back`, held: pickup won't re-pick it until Retry.
    Stop,
    /// Don't do this one → `closed` (`skipped`); never picked up again until Retry.
    Skip,
    /// Mark it done in kwant only → `closed` (`done`).
    Close,
    /// Back to the queue → `picked_up`; starts when a slot is free.
    Retry,
}

impl PersonMove {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "stop" => Self::Stop,
            "skip" => Self::Skip,
            "close" => Self::Close,
            "retry" => Self::Retry,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Skip => "skip",
            Self::Close => "close",
            Self::Retry => "retry",
        }
    }

    /// The states each button works from; anything else is 409 `wrong_state`.
    pub fn from(self) -> &'static [&'static str] {
        match self {
            Self::Stop => &[PICKED_UP, READING, NEEDS_ANSWER, BRIEF_READY, WORKING, IN_REVIEW],
            Self::Skip => &[WAITING, PICKED_UP, READING, NEEDS_ANSWER, BRIEF_READY, TAKEN_BACK, FAILED],
            Self::Close => &[WAITING, PICKED_UP, READING, NEEDS_ANSWER, BRIEF_READY, WORKING, IN_REVIEW, LANDED, TAKEN_BACK, FAILED],
            Self::Retry => &[TAKEN_BACK, CLOSED, FAILED],
        }
    }

    pub fn to(self) -> &'static str {
        match self {
            Self::Stop => TAKEN_BACK,
            Self::Skip | Self::Close => CLOSED,
            Self::Retry => PICKED_UP,
        }
    }

    /// `closed_resolution` for the two closing moves.
    pub fn resolution(self) -> Option<&'static str> {
        match self {
            Self::Skip => Some("skipped"),
            Self::Close => Some("done"),
            _ => None,
        }
    }

    /// The event row's note.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Stop => "stopped by you",
            Self::Skip => "skipped by you",
            Self::Close => "closed by you",
            Self::Retry => "retried by you",
        }
    }
}

/// Whether `from → to` is allowed (api.md §1.4). Moving to the same state is never a move.
pub fn allowed(from: &str, to: &str) -> bool {
    if from == to {
        return false;
    }
    match to {
        // A closed row comes back only through the person's Retry (`PersonMove::from`).
        PICKED_UP => matches!(from, WAITING | TAKEN_BACK | FAILED),
        READING => matches!(from, PICKED_UP | NEEDS_ANSWER),
        NEEDS_ANSWER => from == READING,
        BRIEF_READY => matches!(from, READING | NEEDS_ANSWER),
        WORKING => matches!(from, BRIEF_READY | PICKED_UP),
        IN_REVIEW => matches!(from, WORKING | BRIEF_READY),
        LANDED => matches!(from, IN_REVIEW | WORKING),
        TAKEN_BACK => !matches!(from, CLOSED | LANDED),
        CLOSED => true,
        FAILED => matches!(from, PICKED_UP | READING | NEEDS_ANSWER | BRIEF_READY | WORKING | IN_REVIEW),
        _ => false,
    }
}

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS dev_jira_sites (
        space_id TEXT PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        kind TEXT NOT NULL,
        jira_url TEXT NOT NULL,
        confluence_url TEXT,
        email TEXT,
        auth TEXT NOT NULL,
        cloud_id TEXT,
        me_id TEXT,
        me_name TEXT,
        projects TEXT NOT NULL DEFAULT '[]',
        handover TEXT NOT NULL DEFAULT 'label',
        assignee_id TEXT,
        assignee_key TEXT,
        assignee_name TEXT,
        label TEXT NOT NULL DEFAULT 'kwant',
        jql TEXT,
        team_id TEXT,
        spaces TEXT NOT NULL DEFAULT '[]',
        comment_progress INTEGER NOT NULL DEFAULT 0,
        wait_for_ok INTEGER NOT NULL DEFAULT 0,
        poll_secs INTEGER NOT NULL DEFAULT 300,
        on_ INTEGER NOT NULL DEFAULT 1,
        backfill_done INTEGER NOT NULL DEFAULT 0,
        connected_at TIMESTAMP NOT NULL,
        updated_at TIMESTAMP NOT NULL,
        polled_at TIMESTAMP,
        poll_error TEXT,
        backoff_until TIMESTAMP,
        unreachable_since TIMESTAMP,
        token_rejected_at TIMESTAMP,
        scope_note TEXT,
        project_names TEXT,
        space_names TEXT,
        names_at TIMESTAMP
    )",
    "CREATE TABLE IF NOT EXISTS dev_tickets (
        space_id TEXT NOT NULL,
        key TEXT NOT NULL,
        site_id TEXT NOT NULL,
        issue_id TEXT,
        project TEXT NOT NULL,
        summary TEXT NOT NULL DEFAULT '',
        issue_type TEXT NOT NULL DEFAULT '',
        status TEXT NOT NULL DEFAULT '',
        status_category TEXT NOT NULL DEFAULT 'todo',
        priority TEXT,
        priority_rank INTEGER,
        assignee_id TEXT,
        assignee_name TEXT,
        labels TEXT NOT NULL DEFAULT '[]',
        reporter_id TEXT,
        reporter_name TEXT,
        url TEXT NOT NULL DEFAULT '',
        state TEXT NOT NULL,
        team_id TEXT,
        jira_created TIMESTAMP,
        jira_updated TIMESTAMP,
        handed_over_by TEXT,
        handed_over_at TIMESTAMP,
        asked_comment_id TEXT,
        last_reply_id TEXT,
        start_called_at TIMESTAMP,
        misses INTEGER NOT NULL DEFAULT 0,
        taken_back_at TIMESTAMP,
        taken_back_by TEXT,
        closed_resolution TEXT,
        waiting_since_connect INTEGER NOT NULL DEFAULT 0,
        work_comment_at TIMESTAMP,
        pr_comment_at TIMESTAMP,
        held_by_person INTEGER NOT NULL DEFAULT 0,
        error_code TEXT,
        first_seen_at TIMESTAMP NOT NULL,
        updated_at TIMESTAMP NOT NULL,
        PRIMARY KEY (space_id, key)
    )",
    "CREATE INDEX IF NOT EXISTS dev_tickets_state ON dev_tickets (space_id, state)",
    "CREATE TABLE IF NOT EXISTS dev_ticket_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        space_id TEXT NOT NULL,
        key TEXT NOT NULL,
        kind TEXT NOT NULL,
        from_state TEXT,
        to_state TEXT,
        detail TEXT,
        at TIMESTAMP NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS dev_ticket_events_key ON dev_ticket_events (space_id, key, id)",
];

/// A connected site's settings and poll state (no secret).
#[derive(Debug, Clone, FromRow, serde::Serialize, PartialEq)]
pub struct JiraSiteRow {
    pub space_id: String,
    /// `jira_<hex>`: the vault entry's name, and `TicketRef.site_id`.
    pub id: String,
    /// `cloud` or `dc`.
    pub kind: String,
    pub jira_url: String,
    pub confluence_url: Option<String>,
    pub email: Option<String>,
    /// `basic`, `scoped` or `pat`.
    pub auth: String,
    pub cloud_id: Option<String>,
    pub me_id: Option<String>,
    pub me_name: Option<String>,
    /// JSON array of project keys in the column; an array on the wire (api-decisions 1).
    #[serde(serialize_with = "json_list")]
    pub projects: String,
    /// `label`, `assignee` or `jql`: the same words `PUT jira` takes (api-decisions 2).
    #[serde(serialize_with = "wire_handover")]
    pub handover: String,
    /// What JQL takes: Cloud accountId, DC username.
    pub assignee_id: Option<String>,
    /// What issues report for that user ([`jira::person`](crate::server::dev::atlassian::jira::person)):
    /// Cloud accountId, DC user key. The poller's own match uses this.
    pub assignee_key: Option<String>,
    pub assignee_name: Option<String>,
    pub label: String,
    /// The person's Advanced JQL (scoped by `jql::scoped_custom`); `None` = built from the choices.
    pub jql: Option<String>,
    pub team_id: Option<String>,
    /// JSON array of Confluence space keys; `[]` = Confluence off. An array on the wire.
    #[serde(serialize_with = "json_list")]
    pub spaces: String,
    pub comment_progress: bool,
    pub wait_for_ok: bool,
    pub poll_secs: i64,
    /// `on` on the wire (api-decisions 3).
    #[serde(rename = "on")]
    pub on_: bool,
    /// J5: false until the first poll after connecting (or turning back on) has marked what was
    /// already there as `waiting`.
    pub backfill_done: bool,
    pub connected_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub polled_at: Option<DateTime<Utc>>,
    pub poll_error: Option<String>,
    pub backoff_until: Option<DateTime<Utc>>,
    pub unreachable_since: Option<DateTime<Utc>>,
    pub token_rejected_at: Option<DateTime<Utc>>,
    /// The check's non-secret line for a scoped token ("scoped: reads Jira and Confluence"); `None`
    /// for a classic token or a PAT (api-decisions 6).
    #[sqlx(default)]
    pub scope_note: Option<String>,
    /// `{KEY: name}` for the projects the token sees (api-decisions 23), as a JSON object in the
    /// column and on the wire. Written by the check and the poll only; `PUT` never takes it.
    #[sqlx(default)]
    #[serde(serialize_with = "json_map")]
    pub project_names: Option<String>,
    /// `{KEY: name}` for the Confluence spaces the token sees; same rules as `project_names`.
    #[sqlx(default)]
    #[serde(serialize_with = "json_map")]
    pub space_names: Option<String>,
    /// When the poll last listed the names (it lists them at most every [`NAMES_EVERY_MINUTES`]).
    #[sqlx(default)]
    #[serde(skip)]
    pub names_at: Option<DateTime<Utc>>,
}

/// The poll lists project and space names at most this often (the check always does).
pub const NAMES_EVERY_MINUTES: i64 = 60;

/// A JSON-object column as a real object on the wire (`{}` when empty or unparseable).
fn json_map<S: serde::Serializer>(v: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    use serde::Serialize;
    v.as_deref().and_then(|t| serde_json::from_str::<std::collections::BTreeMap<String, String>>(t).ok()).unwrap_or_default().serialize(s)
}

/// A JSON-array column as a real array on the wire (`[]` when the column doesn't parse).
fn json_list<S: serde::Serializer>(v: &str, s: S) -> Result<S::Ok, S::Error> {
    use serde::Serialize;
    serde_json::from_str::<Vec<String>>(v).unwrap_or_default().serialize(s)
}

/// The stored hand-over, in `PUT jira`'s words only. `PUT` stores nothing else; an unknown value
/// reads as the default `label`.
pub fn handover_word(v: &str) -> &'static str {
    match v {
        "assignee" => "assignee",
        "jql" => "jql",
        _ => "label",
    }
}

fn wire_handover<S: serde::Serializer>(v: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(handover_word(v))
}

impl JiraSiteRow {
    /// A new site's row before its settings are filled in: label `kwant`, 5-minute polls, on.
    pub fn blank(space_id: &str, id: &str) -> Self {
        let now = Utc::now();
        Self {
            space_id: space_id.into(),
            id: id.into(),
            kind: "cloud".into(),
            jira_url: String::new(),
            confluence_url: None,
            email: None,
            auth: "basic".into(),
            cloud_id: None,
            me_id: None,
            me_name: None,
            projects: "[]".into(),
            handover: "label".into(),
            assignee_id: None,
            assignee_key: None,
            assignee_name: None,
            label: "kwant".into(),
            jql: None,
            team_id: None,
            spaces: "[]".into(),
            comment_progress: false,
            wait_for_ok: false,
            poll_secs: 300,
            on_: true,
            backfill_done: false,
            connected_at: now,
            updated_at: now,
            polled_at: None,
            poll_error: None,
            backoff_until: None,
            unreachable_since: None,
            token_rejected_at: None,
            scope_note: None,
            project_names: None,
            space_names: None,
            names_at: None,
        }
    }

    pub fn project_list(&self) -> Vec<String> {
        serde_json::from_str(&self.projects).unwrap_or_default()
    }
    pub fn space_list(&self) -> Vec<String> {
        serde_json::from_str(&self.spaces).unwrap_or_default()
    }
}

/// One picked-up ticket. Fields from Jira are refreshed each poll; `summary` is untrusted text.
#[derive(Debug, Clone, FromRow, serde::Serialize, PartialEq)]
pub struct TicketRow {
    pub space_id: String,
    pub key: String,
    pub site_id: String,
    pub issue_id: Option<String>,
    pub project: String,
    pub summary: String,
    pub issue_type: String,
    pub status: String,
    /// `todo`, `in_progress`, `done`.
    pub status_category: String,
    pub priority: Option<String>,
    pub priority_rank: Option<i64>,
    pub assignee_id: Option<String>,
    pub assignee_name: Option<String>,
    /// JSON array in the column; an array on the wire.
    #[serde(serialize_with = "json_list")]
    pub labels: String,
    pub reporter_id: Option<String>,
    pub reporter_name: Option<String>,
    pub url: String,
    pub state: String,
    pub team_id: Option<String>,
    pub jira_created: Option<DateTime<Utc>>,
    pub jira_updated: Option<DateTime<Utc>>,
    /// `label`, `assignee` (the built rule's choice) or `jql` (the person's Advanced JQL).
    pub handed_over_by: Option<String>,
    pub handed_over_at: Option<DateTime<Utc>>,
    /// The Ask on Jira comment the reporter's reply must follow (Vega sets it).
    pub asked_comment_id: Option<String>,
    /// The newest reporter comment already handed to `hooks::reporter_replied`.
    pub last_reply_id: Option<String>,
    /// When `hooks::start` was last called for this ticket; cleared on a move back to `picked_up`.
    pub start_called_at: Option<DateTime<Utc>>,
    /// Polls in a row a custom-JQL ticket was missing from the search (J14 with Advanced JQL).
    pub misses: i64,
    pub taken_back_at: Option<DateTime<Utc>>,
    /// J14: who took it back in Jira (the display name on the newest change to the assignee or
    /// labels, from the issue's changelog); `None` when Jira didn't say. Untrusted text.
    pub taken_back_by: Option<String>,
    /// `done` (Done in Jira, or the person's Close; the status's name is in `status`), `deleted`
    /// (gone from Jira) or `skipped` (the person's Skip).
    pub closed_resolution: Option<String>,
    /// J5: the ticket already matched when the site was connected (or turned back on), so it came
    /// in as `waiting` for the person's Start. Kept after Start. Waiting cards never count in the
    /// badge (design §11).
    pub waiting_since_connect: bool,
    pub work_comment_at: Option<DateTime<Utc>>,
    pub pr_comment_at: Option<DateTime<Utc>>,
    /// The person pressed Stop: pickup doesn't re-pick this `taken_back` row when it still matches
    /// in Jira, until the person presses Retry (api-decisions 12). Cleared on `picked_up`.
    #[sqlx(default)]
    pub held_by_person: bool,
    /// A `failed` row's reason, one of `tickets::store::FailCode`'s codes; NULL in every other
    /// state. The wire shows `error: {code, message}` with kwant's own sentence for the code
    /// (api-decisions 22): no text from Jira or anywhere else is ever stored here.
    #[sqlx(default)]
    #[serde(skip)]
    pub error_code: Option<String>,
    pub first_seen_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TicketRow {
    pub fn label_list(&self) -> Vec<String> {
        serde_json::from_str(&self.labels).unwrap_or_default()
    }
}

/// What Jira says about a ticket now: written on insert and on every poll.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TicketFacts {
    pub key: String,
    pub issue_id: Option<String>,
    pub project: String,
    pub summary: String,
    pub issue_type: String,
    pub status: String,
    pub status_category: String,
    pub priority: Option<String>,
    pub priority_rank: Option<i64>,
    pub assignee_id: Option<String>,
    pub assignee_name: Option<String>,
    pub labels: Vec<String>,
    pub reporter_id: Option<String>,
    pub reporter_name: Option<String>,
    pub url: String,
    pub jira_created: Option<DateTime<Utc>>,
    pub jira_updated: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize, PartialEq)]
pub struct TicketEventRow {
    pub id: i64,
    pub space_id: String,
    pub key: String,
    /// `state` (a move), `hook` (a hook was called), `comment` (kwant wrote to Jira), `note`.
    pub kind: String,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub detail: Option<String>,
    pub at: DateTime<Utc>,
}

/// The poll outcome written back to the site row.
#[derive(Debug, Clone, Default)]
pub struct PollMark {
    pub error: Option<String>,
    pub backoff_until: Option<DateTime<Utc>>,
    /// The site couldn't be reached at all (network, 5xx): starts or keeps `unreachable_since`.
    pub unreachable: bool,
    /// Jira said 401: the status line says the token stopped working.
    pub token_rejected: bool,
}

impl Database {
    pub(super) async fn migrate_dev_jira(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        // Add-only columns that came after the tables (a database made by an earlier build).
        for (table, column, ddl) in [
            ("dev_jira_sites", "scope_note", "scope_note TEXT"),
            ("dev_tickets", "held_by_person", "held_by_person INTEGER NOT NULL DEFAULT 0"),
            ("dev_jira_sites", "project_names", "project_names TEXT"),
            ("dev_jira_sites", "space_names", "space_names TEXT"),
            ("dev_jira_sites", "names_at", "names_at TIMESTAMP"),
            ("dev_tickets", "error_code", "error_code TEXT"),
        ] {
            let has: Option<(String,)> = sqlx::query_as(&format!("SELECT name FROM pragma_table_info('{table}') WHERE name = ?")).bind(column).fetch_optional(&self.pool).await?;
            if has.is_none() {
                sqlx::query(&format!("ALTER TABLE {table} ADD COLUMN {ddl}")).execute(&self.pool).await?;
            }
        }
        Ok(())
    }

    pub async fn dev_jira_site(&self, space_id: &str) -> Result<Option<JiraSiteRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM dev_jira_sites WHERE space_id = ?").bind(space_id).fetch_optional(&self.pool).await?)
    }

    /// Sites that are on, in spaces with Dev mode on: what pickup polls.
    pub async fn dev_jira_sites_on(&self) -> Result<Vec<JiraSiteRow>, DbError> {
        Ok(sqlx::query_as(
            "SELECT s.* FROM dev_jira_sites s JOIN dev_mode_spaces d ON d.space_id = s.space_id WHERE s.on_ = 1 AND d.on_ = 1 ORDER BY s.space_id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Saves the person's settings (the sheet's Start, or Edit). A new or re-enabled site, or one
    /// whose pickup rule changed, gets `backfill_done = 0` so J5 runs again. Poll state is kept.
    pub async fn save_dev_jira_site(&self, row: &JiraSiteRow, rule_changed: bool) -> Result<JiraSiteRow, DbError> {
        sqlx::query(
            "INSERT INTO dev_jira_sites (space_id, id, kind, jira_url, confluence_url, email, auth, cloud_id, me_id, me_name, projects, handover,
               assignee_id, assignee_name, label, jql, team_id, spaces, comment_progress, wait_for_ok, poll_secs, on_, backfill_done, connected_at, updated_at, assignee_key, scope_note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, 0, ?23, ?23, ?25, ?26)
             ON CONFLICT(space_id) DO UPDATE SET id = excluded.id, kind = excluded.kind, jira_url = excluded.jira_url, confluence_url = excluded.confluence_url,
               email = excluded.email, auth = excluded.auth, cloud_id = excluded.cloud_id, me_id = excluded.me_id, me_name = excluded.me_name,
               projects = excluded.projects, handover = excluded.handover, assignee_id = excluded.assignee_id, assignee_key = excluded.assignee_key, assignee_name = excluded.assignee_name,
               label = excluded.label, jql = excluded.jql, team_id = excluded.team_id, spaces = excluded.spaces, comment_progress = excluded.comment_progress,
               wait_for_ok = excluded.wait_for_ok, poll_secs = excluded.poll_secs, updated_at = excluded.updated_at, scope_note = excluded.scope_note,
               backfill_done = CASE WHEN ?24 OR (excluded.on_ = 1 AND dev_jira_sites.on_ = 0) THEN 0 ELSE dev_jira_sites.backfill_done END,
               on_ = excluded.on_, poll_error = CASE WHEN ?24 THEN NULL ELSE dev_jira_sites.poll_error END,
               token_rejected_at = NULL, backoff_until = NULL, unreachable_since = NULL",
        )
        .bind(&row.space_id)
        .bind(&row.id)
        .bind(&row.kind)
        .bind(&row.jira_url)
        .bind(&row.confluence_url)
        .bind(&row.email)
        .bind(&row.auth)
        .bind(&row.cloud_id)
        .bind(&row.me_id)
        .bind(&row.me_name)
        .bind(&row.projects)
        .bind(&row.handover)
        .bind(&row.assignee_id)
        .bind(&row.assignee_name)
        .bind(&row.label)
        .bind(&row.jql)
        .bind(&row.team_id)
        .bind(&row.spaces)
        .bind(row.comment_progress)
        .bind(row.wait_for_ok)
        .bind(row.poll_secs)
        .bind(row.on_)
        .bind(Utc::now())
        .bind(rule_changed)
        .bind(&row.assignee_key)
        .bind(&row.scope_note)
        .execute(&self.pool)
        .await?;
        self.dev_jira_site(&row.space_id).await?.ok_or_else(|| DbError::Conflict("the Jira site vanished while saving".into()))
    }

    /// Turn off (pickup stops, cards stay) or back on (J5 runs again). Ok(false): no site.
    /// `updated_at` moves only when `on_` changes, so an Off site's row says since when it is Off (D10).
    pub async fn set_dev_jira_on(&self, space_id: &str, on: bool) -> Result<bool, DbError> {
        let done = sqlx::query(
            "UPDATE dev_jira_sites SET on_ = ?2, updated_at = CASE WHEN on_ = ?2 THEN updated_at ELSE ?3 END, backfill_done = CASE WHEN ?2 AND on_ = 0 THEN 0 ELSE backfill_done END,
               backoff_until = NULL, unreachable_since = NULL WHERE space_id = ?1",
        )
        .bind(space_id)
        .bind(on)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn set_dev_jira_backfill_done(&self, space_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_jira_sites SET backfill_done = 1 WHERE space_id = ?").bind(space_id).execute(&self.pool).await?;
        Ok(())
    }

    /// Writes one poll's outcome. A clean poll clears the error, back-off and unreachable clock.
    pub async fn mark_dev_jira_polled(&self, space_id: &str, mark: &PollMark) -> Result<(), DbError> {
        let now = Utc::now();
        sqlx::query(
            "UPDATE dev_jira_sites SET polled_at = ?2, poll_error = ?3, backoff_until = ?4,
               unreachable_since = CASE WHEN ?5 THEN COALESCE(unreachable_since, ?2) ELSE NULL END,
               token_rejected_at = CASE WHEN ?6 THEN COALESCE(token_rejected_at, ?2) ELSE NULL END
             WHERE space_id = ?1",
        )
        .bind(space_id)
        .bind(now)
        .bind(&mark.error)
        .bind(mark.backoff_until)
        .bind(mark.unreachable)
        .bind(mark.token_rejected)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Forgets the site and its tickets and events (the cards' briefs are Vega's: api.md §1.5).
    pub async fn delete_dev_jira_site(&self, space_id: &str) -> Result<Option<JiraSiteRow>, DbError> {
        let site = self.dev_jira_site(space_id).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM dev_ticket_events WHERE space_id = ?").bind(space_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM dev_tickets WHERE space_id = ?").bind(space_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM dev_jira_sites WHERE space_id = ?").bind(space_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(site)
    }

    pub async fn dev_tickets(&self, space_id: &str) -> Result<Vec<TicketRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM dev_tickets WHERE space_id = ? ORDER BY first_seen_at DESC, key").bind(space_id).fetch_all(&self.pool).await?)
    }

    pub async fn dev_ticket(&self, space_id: &str, key: &str) -> Result<Option<TicketRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM dev_tickets WHERE space_id = ? AND key = ?").bind(space_id).bind(key).fetch_optional(&self.pool).await?)
    }

    /// Rows in any of `states`.
    pub async fn dev_tickets_in(&self, space_id: &str, states: &[&str]) -> Result<Vec<TicketRow>, DbError> {
        let all = self.dev_tickets(space_id).await?;
        Ok(all.into_iter().filter(|t| states.contains(&t.state.as_str())).collect())
    }

    /// Adds a ticket in `state` (`waiting` or `picked_up`); Ok(false) when the space already has it.
    pub async fn insert_dev_ticket(&self, space_id: &str, site_id: &str, f: &TicketFacts, state: &str, team_id: Option<&str>, handed_over_by: Option<&str>) -> Result<bool, DbError> {
        if !matches!(state, WAITING | PICKED_UP) {
            return Err(DbError::Conflict(format!("a ticket can't start in {state}")));
        }
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let done = sqlx::query(
            "INSERT OR IGNORE INTO dev_tickets (space_id, key, site_id, issue_id, project, summary, issue_type, status, status_category, priority, priority_rank,
               assignee_id, assignee_name, labels, reporter_id, reporter_name, url, state, team_id, jira_created, jira_updated, handed_over_by, handed_over_at,
               first_seen_at, updated_at, waiting_since_connect)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?23, ?23, ?18 = 'waiting')",
        )
        .bind(space_id)
        .bind(&f.key)
        .bind(site_id)
        .bind(&f.issue_id)
        .bind(&f.project)
        .bind(&f.summary)
        .bind(&f.issue_type)
        .bind(&f.status)
        .bind(&f.status_category)
        .bind(&f.priority)
        .bind(f.priority_rank)
        .bind(&f.assignee_id)
        .bind(&f.assignee_name)
        .bind(serde_json::to_string(&f.labels)?)
        .bind(&f.reporter_id)
        .bind(&f.reporter_name)
        .bind(&f.url)
        .bind(state)
        .bind(team_id)
        .bind(f.jira_created)
        .bind(f.jira_updated)
        .bind(handed_over_by)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(false);
        }
        sqlx::query("INSERT INTO dev_ticket_events (space_id, key, kind, from_state, to_state, detail, at) VALUES (?, ?, 'state', NULL, ?, ?, ?)")
            .bind(space_id)
            .bind(&f.key)
            .bind(state)
            .bind(handed_over_by.map(|h| format!("handed over by {h}")))
            .bind(now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Refreshes what Jira says (never the state).
    pub async fn update_dev_ticket_facts(&self, space_id: &str, f: &TicketFacts) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE dev_tickets SET issue_id = COALESCE(?3, issue_id), summary = ?4, issue_type = ?5, status = ?6, status_category = ?7, priority = ?8,
               priority_rank = ?9, assignee_id = ?10, assignee_name = ?11, labels = ?12, reporter_id = ?13, reporter_name = ?14,
               jira_created = COALESCE(?15, jira_created), jira_updated = COALESCE(?16, jira_updated), url = CASE WHEN ?17 = '' THEN url ELSE ?17 END,
               updated_at = ?18
             WHERE space_id = ?1 AND key = ?2",
        )
        .bind(space_id)
        .bind(&f.key)
        .bind(&f.issue_id)
        .bind(&f.summary)
        .bind(&f.issue_type)
        .bind(&f.status)
        .bind(&f.status_category)
        .bind(&f.priority)
        .bind(f.priority_rank)
        .bind(&f.assignee_id)
        .bind(&f.assignee_name)
        .bind(serde_json::to_string(&f.labels)?)
        .bind(&f.reporter_id)
        .bind(&f.reporter_name)
        .bind(f.jira_created)
        .bind(f.jira_updated)
        .bind(&f.url)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The one writer of a ticket's state (api.md §1.4). Ok(false): no such ticket, or the move
    /// isn't allowed from its current state (a late `brief_ready` never overwrites `taken_back`).
    /// The check and the write are one statement, so two writers can't both win.
    pub async fn set_dev_ticket_state(&self, space_id: &str, key: &str, to: &str) -> Result<bool, DbError> {
        self.set_dev_ticket_state_with(space_id, key, to, None).await
    }

    /// [`set_dev_ticket_state`](Self::set_dev_ticket_state) with a note on the event row
    /// (`"unassigned"`, `"done: Done"`, `"deleted"`).
    pub async fn set_dev_ticket_state_with(&self, space_id: &str, key: &str, to: &str, detail: Option<&str>) -> Result<bool, DbError> {
        let resolution = detail.filter(|_| to == CLOSED).map(|d| if d.starts_with("deleted") { "deleted" } else { "done" });
        let code = (to == FAILED).then_some(FAIL_DEFAULT_CODE);
        Ok(self.write_state(space_id, key, to, detail, None, resolution, false, code).await?.is_some())
    }

    /// Moves the ticket to `failed` with `code` (a `tickets::store::FailCode` code; the caller
    /// passes the enum's code, never free text) and a note on the event row.
    pub async fn fail_dev_ticket(&self, space_id: &str, key: &str, code: &'static str, detail: Option<&str>) -> Result<bool, DbError> {
        Ok(self.write_state(space_id, key, FAILED, detail, None, None, false, Some(code)).await?.is_some())
    }

    /// Replaces the names the check or the poll listed (`None` leaves that list as it is) and
    /// stamps `names_at`. Keys must look like keys; names are cut to 120 characters.
    pub async fn set_dev_jira_names(&self, space_id: &str, projects: Option<&[(String, String)]>, spaces: Option<&[(String, String)]>) -> Result<(), DbError> {
        let enc = |l: Option<&[(String, String)]>| {
            l.map(|l| {
                let m: std::collections::BTreeMap<&str, String> = l
                    .iter()
                    .filter(|(k, _)| !k.is_empty() && k.len() <= 255 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '~' || c == '-'))
                    .map(|(k, n)| (k.as_str(), n.chars().filter(|c| !c.is_control()).take(120).collect::<String>().trim().to_string()))
                    .collect();
                serde_json::to_string(&m).unwrap_or_else(|_| "{}".into())
            })
        };
        sqlx::query("UPDATE dev_jira_sites SET project_names = COALESCE(?2, project_names), space_names = COALESCE(?3, space_names), names_at = ?4 WHERE space_id = ?1")
            .bind(space_id)
            .bind(enc(projects))
            .bind(enc(spaces))
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The person's Stop, Skip, Close or Retry (api-decisions 12): the same writer, limited to
    /// [`PersonMove::from`]. Ok(Some(from)) when it moved; Ok(None) when there's no such ticket or
    /// it can't move from where it is (the route's 409 `wrong_state`). Nothing is written to Jira.
    pub async fn person_move_dev_ticket(&self, space_id: &str, key: &str, mv: PersonMove) -> Result<Option<String>, DbError> {
        self.write_state(space_id, key, mv.to(), Some(mv.detail()), Some(mv.from()), mv.resolution(), mv == PersonMove::Stop, None).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_state(&self, space_id: &str, key: &str, to: &str, detail: Option<&str>, only_from: Option<&[&str]>, resolution: Option<&str>, hold: bool, error_code: Option<&str>) -> Result<Option<String>, DbError> {
        if !STATES.contains(&to) {
            return Err(DbError::Conflict(format!("{to} isn't a ticket state")));
        }
        let from: Vec<&str> = STATES.iter().copied().filter(|f| allowed(f, to)).collect();
        if from.is_empty() {
            return Ok(None);
        }
        let now = Utc::now();
        // It reads before it writes: take the write lock at BEGIN, or a brief turn and the poller
        // moving two tickets at once fail the second write with "database is locked"
        let mut tx = self.begin_write().await?;
        let Some((current,)) = sqlx::query_as::<_, (String,)>("SELECT state FROM dev_tickets WHERE space_id = ? AND key = ?")
            .bind(space_id)
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?
        else {
            tx.rollback().await?;
            return Ok(None);
        };
        // A person's move is limited to its own table (`PersonMove::from`), which may reopen a
        // closed row (Retry); every other writer to `allowed`.
        let ok = match only_from {
            Some(f) => current != to && f.contains(&current.as_str()),
            None => allowed(&current, to),
        };
        if !ok {
            tx.rollback().await?;
            return Ok(None);
        }
        let done = sqlx::query(
            "UPDATE dev_tickets SET state = ?3, updated_at = ?4,
               start_called_at = CASE WHEN ?3 = 'picked_up' THEN NULL ELSE start_called_at END,
               misses = 0,
               taken_back_at = CASE WHEN ?3 = 'taken_back' THEN ?4 WHEN ?3 = 'picked_up' THEN NULL ELSE taken_back_at END,
               taken_back_by = CASE WHEN ?3 IN ('taken_back', 'picked_up') THEN NULL ELSE taken_back_by END,
               closed_resolution = CASE WHEN ?3 = 'closed' THEN COALESCE(?6, 'done') WHEN ?3 = 'picked_up' THEN NULL ELSE closed_resolution END,
               held_by_person = CASE WHEN ?7 THEN 1 WHEN ?3 = 'picked_up' THEN 0 ELSE held_by_person END,
               error_code = CASE WHEN ?3 = 'failed' THEN COALESCE(?8, 'failed') ELSE NULL END
             WHERE space_id = ?1 AND key = ?2 AND state = ?5",
        )
        .bind(space_id)
        .bind(key)
        .bind(to)
        .bind(now)
        .bind(&current)
        .bind(resolution)
        .bind(hold)
        .bind(error_code)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        sqlx::query("INSERT INTO dev_ticket_events (space_id, key, kind, from_state, to_state, detail, at) VALUES (?, ?, 'state', ?, ?, ?, ?)")
            .bind(space_id)
            .bind(key)
            .bind(&current)
            .bind(to)
            .bind(detail)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        // The brief row's `state` mirrors the card (D2). Same transaction: if the mirror fails the
        // move is rolled back, so the brief page and Vega's hooks never disagree with the board
        // (Rigel's #452 note). A ticket without a brief row moves the same.
        super::dev_ticket_briefs::move_brief(&mut tx, space_id, key, STATES, to).await?;
        tx.commit().await?;
        Ok(Some(current))
    }

    /// The Ask on Jira comment the reporter's reply must follow (Vega's route, api.md §1.3).
    /// Resets `last_reply_id`, so only replies after this comment count.
    pub async fn set_dev_ticket_asked(&self, space_id: &str, key: &str, comment_id: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE dev_tickets SET asked_comment_id = ?3, last_reply_id = NULL, updated_at = ?4 WHERE space_id = ?1 AND key = ?2")
            .bind(space_id)
            .bind(key)
            .bind(comment_id)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        if done.rows_affected() > 0 {
            self.add_dev_ticket_event(space_id, key, "comment", Some(&format!("asked on Jira: comment {comment_id}"))).await?;
        }
        Ok(done.rows_affected() > 0)
    }

    /// J14: who took the ticket back (set after the move to `taken_back`; display name only).
    pub async fn set_dev_ticket_taken_back_by(&self, space_id: &str, key: &str, name: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_tickets SET taken_back_by = ?3 WHERE space_id = ?1 AND key = ?2 AND state = 'taken_back'")
            .bind(space_id)
            .bind(key)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_dev_ticket_last_reply(&self, space_id: &str, key: &str, comment_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_tickets SET last_reply_id = ?3 WHERE space_id = ?1 AND key = ?2").bind(space_id).bind(key).bind(comment_id).execute(&self.pool).await?;
        Ok(())
    }

    /// Marks `hooks::start` called; Ok(false) when it already was (two polls can't start it twice).
    pub async fn claim_dev_ticket_start(&self, space_id: &str, key: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE dev_tickets SET start_called_at = ?3 WHERE space_id = ?1 AND key = ?2 AND state = 'picked_up' AND start_called_at IS NULL")
            .bind(space_id)
            .bind(key)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// D10: `picked_up` cards whose start was claimed at or after `since` (the site was Off then,
    /// so `hooks::start` left them as they were) join the queue again. Answers their keys.
    pub async fn requeue_dev_tickets_started_since(&self, space_id: &str, since: DateTime<Utc>) -> Result<Vec<String>, DbError> {
        let keys: Vec<(String,)> = sqlx::query_as(
            "UPDATE dev_tickets SET start_called_at = NULL WHERE space_id = ?1 AND state = 'picked_up' AND start_called_at IS NOT NULL AND start_called_at >= ?2 RETURNING key",
        )
        .bind(space_id)
        .bind(since)
        .fetch_all(&self.pool)
        .await?;
        Ok(keys.into_iter().map(|(k,)| k).collect())
    }

    /// One more poll a custom-JQL ticket was missing from; returns the new count.
    pub async fn bump_dev_ticket_misses(&self, space_id: &str, key: &str) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("UPDATE dev_tickets SET misses = misses + 1 WHERE space_id = ? AND key = ? RETURNING misses").bind(space_id).bind(key).fetch_one(&self.pool).await?;
        Ok(n)
    }

    pub async fn reset_dev_ticket_misses(&self, space_id: &str, key: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_tickets SET misses = 0 WHERE space_id = ? AND key = ? AND misses <> 0").bind(space_id).bind(key).execute(&self.pool).await?;
        Ok(())
    }

    /// J9: claims the one work-started (`work`) or PR-ready (`pr`) comment; Ok(false) if already made.
    pub async fn claim_dev_ticket_comment(&self, space_id: &str, key: &str, which: &str) -> Result<bool, DbError> {
        let col = match which {
            "work" => "work_comment_at",
            "pr" => "pr_comment_at",
            _ => return Err(DbError::Conflict("unknown progress comment".into())),
        };
        let done = sqlx::query(&format!("UPDATE dev_tickets SET {col} = ?3 WHERE space_id = ?1 AND key = ?2 AND {col} IS NULL"))
            .bind(space_id)
            .bind(key)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn add_dev_ticket_event(&self, space_id: &str, key: &str, kind: &str, detail: Option<&str>) -> Result<(), DbError> {
        sqlx::query("INSERT INTO dev_ticket_events (space_id, key, kind, detail, at) VALUES (?, ?, ?, ?, ?)")
            .bind(space_id)
            .bind(key)
            .bind(kind)
            .bind(detail)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The newest `limit` events, oldest first.
    pub async fn dev_ticket_events(&self, space_id: &str, key: &str, limit: i64) -> Result<Vec<TicketEventRow>, DbError> {
        let mut rows: Vec<TicketEventRow> = sqlx::query_as("SELECT * FROM dev_ticket_events WHERE space_id = ? AND key = ? ORDER BY id DESC LIMIT ?")
            .bind(space_id)
            .bind(key)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.reverse();
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn many_tickets_moving_at_once_all_move() {
        // Brief turns and the poller move different tickets at the same moment: each move reads
        // the row, then writes, so it must hold the write lock from BEGIN (a deferred transaction
        // fails at once with "database is locked" when another write lands between)
        let dir = tempfile::tempdir().unwrap();
        let db = Database::init_with_busy_timeout(dir.path(), std::time::Duration::from_secs(60)).await.unwrap();
        let keys: Vec<String> = (1..=12).map(|i| format!("TRAIL-{i}")).collect();
        for k in &keys {
            let f = TicketFacts { key: k.clone(), project: "TRAIL".into(), url: format!("https://acme.atlassian.net/browse/{k}"), ..Default::default() };
            assert!(db.insert_dev_ticket("s1", "site1", &f, PICKED_UP, Some("team1"), Some("label")).await.unwrap());
        }
        let mut handles = Vec::new();
        for k in keys.clone() {
            let db = db.clone();
            handles.push(tokio::spawn(async move {
                for to in [READING, BRIEF_READY, WORKING] {
                    assert!(db.set_dev_ticket_state("s1", &k, to).await.unwrap(), "{k} to {to}");
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        for k in &keys {
            assert_eq!(db.dev_ticket("s1", k).await.unwrap().unwrap().state, WORKING);
            assert_eq!(db.dev_ticket_events("s1", k, 10).await.unwrap().iter().filter(|e| e.kind == "state").count(), 4, "{k}: one event per move");
        }
        // A refused move leaves the lock free for the next writer
        assert!(!db.set_dev_ticket_state("s1", "TRAIL-1", PICKED_UP).await.unwrap());
        assert!(db.set_dev_ticket_state("s1", "TRAIL-2", IN_REVIEW).await.unwrap());
    }

    #[tokio::test]
    async fn a_move_and_its_brief_mirror_are_one_transaction() {
        // Rigel's #452 note: the card and its brief row moved in two writes, so a failed mirror
        // left them disagreeing. Now a refused brief write rolls the move back.
        let dir = tempfile::tempdir().unwrap();
        let db = Database::init(dir.path()).await.unwrap();
        for k in ["MIR-1", "MIR-2"] {
            let f = TicketFacts { key: k.into(), project: "MIR".into(), url: format!("https://acme.atlassian.net/browse/{k}"), ..Default::default() };
            assert!(db.insert_dev_ticket("s1", "site1", &f, PICKED_UP, Some("team1"), Some("label")).await.unwrap());
        }
        db.upsert_ticket_brief("s1", "MIR-1", "site1", "team1", "https://acme.atlassian.net/browse/MIR-1", "one").await.unwrap();

        // Each writer moves the brief row with the card
        assert!(db.set_dev_ticket_state("s1", "MIR-1", READING).await.unwrap());
        assert_eq!(db.ticket_brief("s1", "MIR-1").await.unwrap().unwrap().state, READING);
        assert!(db.fail_dev_ticket("s1", "MIR-1", FAIL_DEFAULT_CODE, Some("x")).await.unwrap());
        assert_eq!(db.ticket_brief("s1", "MIR-1").await.unwrap().unwrap().state, FAILED);
        assert!(db.person_move_dev_ticket("s1", "MIR-1", PersonMove::Retry).await.unwrap().is_some());
        let retried = db.dev_ticket("s1", "MIR-1").await.unwrap().unwrap().state;
        assert_eq!(db.ticket_brief("s1", "MIR-1").await.unwrap().unwrap().state, retried);
        // A ticket without a brief row moves as before
        assert!(db.set_dev_ticket_state("s1", "MIR-2", READING).await.unwrap());

        // The brief write fails: the move is undone, card, event log and brief alike
        sqlx::query("CREATE TRIGGER refuse_brief BEFORE UPDATE OF state ON dev_ticket_briefs BEGIN SELECT RAISE(ABORT, 'refused'); END")
            .execute(&db.pool)
            .await
            .unwrap();
        let events = db.dev_ticket_events("s1", "MIR-1", 50).await.unwrap().len();
        assert!(db.set_dev_ticket_state("s1", "MIR-1", READING).await.is_err(), "the failed mirror is the move's error");
        assert_eq!(db.dev_ticket("s1", "MIR-1").await.unwrap().unwrap().state, retried, "the card didn't move");
        assert_eq!(db.ticket_brief("s1", "MIR-1").await.unwrap().unwrap().state, retried);
        assert_eq!(db.dev_ticket_events("s1", "MIR-1", 50).await.unwrap().len(), events, "no event for a move that didn't happen");
        assert!(db.fail_dev_ticket("s1", "MIR-1", FAIL_DEFAULT_CODE, None).await.is_err());
        assert_eq!(db.dev_ticket("s1", "MIR-1").await.unwrap().unwrap().state, retried);
        // The trigger only fires for a row that exists: other tickets still move, and the lock is free
        assert!(db.set_dev_ticket_state("s1", "MIR-2", BRIEF_READY).await.unwrap());
    }

    #[test]
    fn the_persons_moves_are_api_md_1_4() {
        let all = [PersonMove::Stop, PersonMove::Skip, PersonMove::Close, PersonMove::Retry];
        for mv in all {
            assert_eq!(PersonMove::parse(mv.as_str()), Some(mv));
            assert!(!mv.from().contains(&mv.to()), "{} never moves a row to where it is", mv.as_str());
            for f in mv.from() {
                // Each move is one the state table allows, but Retry may also reopen a closed row
                assert!(allowed(f, mv.to()) || (mv == PersonMove::Retry && *f == CLOSED), "{} from {f}", mv.as_str());
            }
        }
        assert_eq!(PersonMove::parse("delete"), None);
        assert!(!PersonMove::Stop.from().contains(&WAITING), "a waiting row has nothing to stop");
        assert!(!PersonMove::Skip.from().iter().any(|f| matches!(*f, WORKING | IN_REVIEW | LANDED)), "work under way is stopped or closed, not skipped");
        assert!(!PersonMove::Retry.from().iter().any(|f| matches!(*f, WAITING | WORKING | LANDED)));
        assert_eq!((PersonMove::Skip.resolution(), PersonMove::Close.resolution(), PersonMove::Stop.resolution()), (Some("skipped"), Some("done"), None));
    }

    #[test]
    fn the_state_table_is_api_md_1_4() {
        assert!(allowed(WAITING, PICKED_UP) && allowed(TAKEN_BACK, PICKED_UP) && allowed(FAILED, PICKED_UP));
        assert!(!allowed(READING, PICKED_UP));
        assert!(allowed(PICKED_UP, READING) && allowed(NEEDS_ANSWER, READING));
        assert!(allowed(READING, NEEDS_ANSWER) && !allowed(PICKED_UP, NEEDS_ANSWER));
        assert!(allowed(READING, BRIEF_READY) && allowed(NEEDS_ANSWER, BRIEF_READY) && !allowed(TAKEN_BACK, BRIEF_READY));
        assert!(allowed(BRIEF_READY, WORKING) && allowed(PICKED_UP, WORKING) && !allowed(READING, WORKING));
        assert!(allowed(WORKING, IN_REVIEW) && allowed(BRIEF_READY, IN_REVIEW) && !allowed(PICKED_UP, IN_REVIEW));
        assert!(allowed(IN_REVIEW, LANDED) && allowed(WORKING, LANDED) && !allowed(BRIEF_READY, LANDED));
        for s in STATES {
            assert_eq!(allowed(s, TAKEN_BACK), !matches!(*s, CLOSED | LANDED | TAKEN_BACK), "{s} → taken_back");
            assert_eq!(allowed(s, CLOSED), *s != CLOSED, "{s} → closed");
            assert_eq!(allowed(s, PICKED_UP), matches!(*s, WAITING | TAKEN_BACK | FAILED), "{s} → picked_up: a closed row never comes back by itself");
            assert!(!allowed(s, WAITING), "nothing moves back to waiting");
        }
        assert!(allowed(WORKING, FAILED) && !allowed(WAITING, FAILED) && !allowed(CLOSED, FAILED) && !allowed(LANDED, FAILED));
    }
}
