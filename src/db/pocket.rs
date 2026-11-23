//! Pocket apps (pa1): small live apps guests join from their phones with no account. All tables
//! are add-only and nothing here runs unless the `pocket_apps` setting is `on` (see
//! `crate::pocket`). There are no foreign keys (`foreign_keys` is on for one pooled connection
//! only), so deleting a pocket deletes its players, keys and events in one transaction.
//!
//! What is never stored: a PIN or a reset code (only an argon2 hash with a per-player salt), a
//! guest or host key (only its sha256), and any IP address (the limiter keeps those in memory).

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

/// A pocket that guests can join and play.
pub const OPEN: &str = "open";
/// Ended by the host (or its closing time): read-only until its data expires.
pub const ENDED: &str = "ended";

/// Guest keys one player may hold at once (phones, a laptop); the oldest goes first.
pub const MAX_KEYS_PER_PLAYER: i64 = 5;

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct PocketRow {
    pub code: String,
    pub kit: String,
    pub title: String,
    pub spec: String,
    pub kit_state: String,
    pub state: String,
    pub version: i64,
    pub owner_kwant_id: Option<String>,
    pub opens_at: Option<DateTime<Utc>>,
    pub closes_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub host_key_hash: String,
    pub created_at: DateTime<Utc>,
}

/// One guest of a pocket. Never sent anywhere as is: the routes pick the public fields.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct PlayerRow {
    pub pocket_code: String,
    pub id: String,
    pub name: String,
    pub name_key: String,
    pub avatar: String,
    pub team: Option<String>,
    pub pin_hash: Option<String>,
    pub pin_salt: Option<String>,
    /// Wrong tries since the last right one or the last 10-minute lock.
    pub wrong_tries: i64,
    /// Wrong tries in total; the 10-minute wait doesn't reset it, a host's reset code does.
    pub total_wrong: i64,
    pub locked_until: Option<DateTime<Utc>>,
    /// Locked until the host makes a reset code (10 wrong tries in total).
    pub pin_locked: bool,
    /// The last wrong try nobody has been told about yet.
    pub last_wrong_at: Option<DateTime<Utc>>,
    pub reset_code_hash: Option<String>,
    pub reset_expires_at: Option<DateTime<Utc>>,
    /// The host's last PIN reset nobody has been told about yet.
    pub pin_reset_at: Option<DateTime<Utc>>,
    pub joined_at: DateTime<Utc>,
    pub removed_at: Option<DateTime<Utc>>,
}

/// The counts the main port's pocket list shows (no names, no guest text).
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct PocketCounts {
    pub players: i64,
    pub removed: i64,
}

const POCKET_COLS: &str = "code, kit, title, spec, kit_state, state, version, owner_kwant_id, opens_at, closes_at, \
    ended_at, expires_at, host_key_hash, created_at";
const PLAYER_COLS: &str = "pocket_code, id, name, name_key, avatar, team, pin_hash, pin_salt, wrong_tries, \
    total_wrong, locked_until, pin_locked, last_wrong_at, reset_code_hash, reset_expires_at, pin_reset_at, \
    joined_at, removed_at";

/// A new pocket as `crate::pocket::create` makes it.
pub struct NewPocket<'a> {
    pub code: &'a str,
    pub kit: &'a str,
    pub title: &'a str,
    pub spec: &'a str,
    pub kit_state: &'a str,
    pub owner_kwant_id: Option<&'a str>,
    pub opens_at: Option<DateTime<Utc>>,
    pub closes_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub host_key_hash: &'a str,
    pub now: DateTime<Utc>,
}

/// A new player as `crate::pocket::join` makes it.
pub struct NewPlayer<'a> {
    pub pocket_code: &'a str,
    pub id: &'a str,
    pub name: &'a str,
    pub name_key: &'a str,
    pub avatar: &'a str,
    pub team: Option<&'a str>,
    pub pin_hash: Option<&'a str>,
    pub pin_salt: Option<&'a str>,
    pub now: DateTime<Utc>,
}

/// How a PIN try changes a player's counters (`crate::pocket::pin` decides).
#[derive(Debug, Clone, PartialEq)]
pub struct PinCounters {
    pub wrong_tries: i64,
    pub total_wrong: i64,
    pub locked_until: Option<DateTime<Utc>>,
    pub pin_locked: bool,
    pub last_wrong_at: Option<DateTime<Utc>>,
}

impl Database {
    pub(super) async fn migrate_pocket(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS pocket_apps (
                code TEXT PRIMARY KEY,
                kit TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                spec TEXT NOT NULL,
                kit_state TEXT NOT NULL DEFAULT '{}',
                state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'ended')),
                version INTEGER NOT NULL DEFAULT 0,
                owner_kwant_id TEXT,
                opens_at TEXT,
                closes_at TEXT,
                ended_at TEXT,
                expires_at TEXT,
                host_key_hash TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS pocket_apps_expires ON pocket_apps(expires_at)",
            "CREATE TABLE IF NOT EXISTS pocket_players (
                pocket_code TEXT NOT NULL,
                id TEXT NOT NULL,
                name TEXT NOT NULL,
                name_key TEXT NOT NULL,
                avatar TEXT NOT NULL DEFAULT '',
                team TEXT,
                pin_hash TEXT,
                pin_salt TEXT,
                wrong_tries INTEGER NOT NULL DEFAULT 0,
                total_wrong INTEGER NOT NULL DEFAULT 0,
                locked_until TEXT,
                pin_locked INTEGER NOT NULL DEFAULT 0,
                last_wrong_at TEXT,
                reset_code_hash TEXT,
                reset_expires_at TEXT,
                pin_reset_at TEXT,
                joined_at TEXT NOT NULL,
                removed_at TEXT,
                PRIMARY KEY (pocket_code, id)
            )",
            // Names are unique per pocket, trimmed and case-insensitive (removed players keep theirs)
            "CREATE UNIQUE INDEX IF NOT EXISTS pocket_players_name ON pocket_players(pocket_code, name_key)",
            // A guest key belongs to one player of one pocket; only its sha256 is kept
            "CREATE TABLE IF NOT EXISTS pocket_guest_keys (
                key_hash TEXT PRIMARY KEY,
                pocket_code TEXT NOT NULL,
                player_id TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS pocket_guest_keys_player ON pocket_guest_keys(pocket_code, player_id)",
            "CREATE TABLE IF NOT EXISTS pocket_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pocket_code TEXT NOT NULL,
                player_id TEXT,
                kind TEXT NOT NULL,
                payload TEXT NOT NULL DEFAULT '{}',
                at TEXT NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS pocket_events_pocket ON pocket_events(pocket_code, id)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn pocket_insert(&self, p: NewPocket<'_>) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO pocket_apps (code, kit, title, spec, kit_state, state, version, owner_kwant_id, opens_at, \
             closes_at, expires_at, host_key_hash, created_at) VALUES (?, ?, ?, ?, ?, 'open', 1, ?, ?, ?, ?, ?, ?)",
        )
        .bind(p.code)
        .bind(p.kit)
        .bind(p.title)
        .bind(p.spec)
        .bind(p.kit_state)
        .bind(p.owner_kwant_id)
        .bind(p.opens_at)
        .bind(p.closes_at)
        .bind(p.expires_at)
        .bind(p.host_key_hash)
        .bind(p.now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn pocket_get(&self, code: &str) -> Result<Option<PocketRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {POCKET_COLS} FROM pocket_apps WHERE code = ?"))
            .bind(code)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Newest first.
    pub async fn pocket_list(&self) -> Result<Vec<PocketRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {POCKET_COLS} FROM pocket_apps ORDER BY created_at DESC, code")).fetch_all(&self.pool).await?)
    }

    pub async fn pocket_codes_open(&self) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT code FROM pocket_apps WHERE state = 'open'").fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    pub async fn pocket_counts(&self, code: &str) -> Result<PocketCounts, DbError> {
        let (players, removed): (i64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(removed_at IS NULL), 0), COALESCE(SUM(removed_at IS NOT NULL), 0) \
             FROM pocket_players WHERE pocket_code = ?",
        )
        .bind(code)
        .fetch_one(&self.pool)
        .await?;
        Ok(PocketCounts { players, removed })
    }

    /// Saves the kit's state and its events, and bumps the version, in one transaction.
    pub async fn pocket_save_state(&self, code: &str, kit_state: &str, events: &[(Option<String>, String, String)], now: DateTime<Utc>) -> Result<i64, DbError> {
        let mut tx = self.pool.begin().await?;
        let (version,): (i64,) = sqlx::query_as("UPDATE pocket_apps SET kit_state = ?, version = version + 1 WHERE code = ? RETURNING version")
            .bind(kit_state)
            .bind(code)
            .fetch_one(&mut *tx)
            .await?;
        for (player, kind, payload) in events {
            sqlx::query("INSERT INTO pocket_events (pocket_code, player_id, kind, payload, at) VALUES (?, ?, ?, ?, ?)")
                .bind(code)
                .bind(player)
                .bind(kind)
                .bind(payload)
                .bind(now)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(version)
    }

    pub async fn pocket_bump(&self, code: &str) -> Result<i64, DbError> {
        let (v,): (i64,) = sqlx::query_as("UPDATE pocket_apps SET version = version + 1 WHERE code = ? RETURNING version")
            .bind(code)
            .fetch_one(&self.pool)
            .await?;
        Ok(v)
    }

    /// Ends an open pocket; `false` when it was already ended (or is gone).
    pub async fn pocket_end(&self, code: &str, now: DateTime<Utc>, expires_at: Option<DateTime<Utc>>) -> Result<bool, DbError> {
        let done = sqlx::query(
            "UPDATE pocket_apps SET state = 'ended', ended_at = ?, expires_at = ?, version = version + 1 \
             WHERE code = ? AND state = 'open'",
        )
        .bind(now)
        .bind(expires_at)
        .bind(code)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn pocket_set_host_key(&self, code: &str, host_key_hash: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE pocket_apps SET host_key_hash = ? WHERE code = ?").bind(host_key_hash).bind(code).execute(&self.pool).await?;
        Ok(done.rows_affected() > 0)
    }

    /// Deletes the pocket, its players, keys and events. Final: there is no undo.
    pub async fn pocket_delete(&self, code: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        for table in ["pocket_events", "pocket_guest_keys", "pocket_players"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE pocket_code = ?")).bind(code).execute(&mut *tx).await?;
        }
        let done = sqlx::query("DELETE FROM pocket_apps WHERE code = ?").bind(code).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(done.rows_affected() > 0)
    }

    /// Codes of pockets whose data has expired by `now`.
    pub async fn pocket_expired(&self, now: DateTime<Utc>) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT code FROM pocket_apps WHERE expires_at IS NOT NULL AND expires_at <= ?")
            .bind(now)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Open pockets whose closing time has passed by `now`.
    pub async fn pocket_past_closing(&self, now: DateTime<Utc>) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT code FROM pocket_apps WHERE state = 'open' AND closes_at IS NOT NULL AND closes_at <= ?")
            .bind(now)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    // Players

    pub async fn pocket_player_insert(&self, p: NewPlayer<'_>) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO pocket_players (pocket_code, id, name, name_key, avatar, team, pin_hash, pin_salt, joined_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(p.pocket_code)
        .bind(p.id)
        .bind(p.name)
        .bind(p.name_key)
        .bind(p.avatar)
        .bind(p.team)
        .bind(p.pin_hash)
        .bind(p.pin_salt)
        .bind(p.now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn pocket_player(&self, code: &str, id: &str) -> Result<Option<PlayerRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {PLAYER_COLS} FROM pocket_players WHERE pocket_code = ? AND id = ?"))
            .bind(code)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn pocket_player_by_name(&self, code: &str, name_key: &str) -> Result<Option<PlayerRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {PLAYER_COLS} FROM pocket_players WHERE pocket_code = ? AND name_key = ?"))
            .bind(code)
            .bind(name_key)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Players still in the pocket, in join order.
    pub async fn pocket_players(&self, code: &str) -> Result<Vec<PlayerRow>, DbError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {PLAYER_COLS} FROM pocket_players WHERE pocket_code = ? AND removed_at IS NULL ORDER BY joined_at, id"
        ))
        .bind(code)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn pocket_player_remove(&self, code: &str, id: &str, now: DateTime<Utc>) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let done = sqlx::query("UPDATE pocket_players SET removed_at = ? WHERE pocket_code = ? AND id = ? AND removed_at IS NULL")
            .bind(now)
            .bind(code)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM pocket_guest_keys WHERE pocket_code = ? AND player_id = ?").bind(code).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn pocket_set_pin_counters(&self, code: &str, id: &str, c: &PinCounters) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE pocket_players SET wrong_tries = ?, total_wrong = ?, locked_until = ?, pin_locked = ?, last_wrong_at = ? \
             WHERE pocket_code = ? AND id = ?",
        )
        .bind(c.wrong_tries)
        .bind(c.total_wrong)
        .bind(c.locked_until)
        .bind(c.pin_locked)
        .bind(c.last_wrong_at)
        .bind(code)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A right PIN (or reset code + new PIN): the short counter clears, the notices were told.
    pub async fn pocket_pin_ok(&self, code: &str, id: &str, new_pin: Option<(&str, &str)>) -> Result<(), DbError> {
        match new_pin {
            Some((hash, salt)) => {
                sqlx::query(
                    "UPDATE pocket_players SET pin_hash = ?, pin_salt = ?, reset_code_hash = NULL, reset_expires_at = NULL, \
                     wrong_tries = 0, locked_until = NULL, last_wrong_at = NULL, pin_reset_at = NULL WHERE pocket_code = ? AND id = ?",
                )
                .bind(hash)
                .bind(salt)
                .bind(code)
                .bind(id)
                .execute(&self.pool)
                .await?;
            }
            None => {
                sqlx::query(
                    "UPDATE pocket_players SET wrong_tries = 0, locked_until = NULL, last_wrong_at = NULL, pin_reset_at = NULL \
                     WHERE pocket_code = ? AND id = ?",
                )
                .bind(code)
                .bind(id)
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }

    /// The player has seen their notices (a wrong PIN try, the host's reset); the counters stay.
    pub async fn pocket_notices_seen(&self, code: &str, id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE pocket_players SET last_wrong_at = NULL, pin_reset_at = NULL WHERE pocket_code = ? AND id = ?")
            .bind(code)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The host's reset: the old PIN and every guest key of the player stop working, the locks
    /// and counters clear, and a one-time code (its hash) waits until `expires_at`. The salt is
    /// kept: the code is hashed with it.
    pub async fn pocket_pin_reset(&self, code: &str, id: &str, code_hash: &str, expires_at: DateTime<Utc>, now: DateTime<Utc>) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let done = sqlx::query(
            "UPDATE pocket_players SET pin_hash = NULL, reset_code_hash = ?, reset_expires_at = ?, pin_reset_at = ?, \
             wrong_tries = 0, total_wrong = 0, locked_until = NULL, pin_locked = 0 \
             WHERE pocket_code = ? AND id = ? AND removed_at IS NULL",
        )
        .bind(code_hash)
        .bind(expires_at)
        .bind(now)
        .bind(code)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM pocket_guest_keys WHERE pocket_code = ? AND player_id = ?").bind(code).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(done.rows_affected() > 0)
    }

    /// Sets the per-player salt when the player has none yet (players of kits without a PIN).
    pub async fn pocket_set_salt(&self, code: &str, id: &str, salt: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE pocket_players SET pin_salt = ? WHERE pocket_code = ? AND id = ? AND pin_salt IS NULL")
            .bind(salt)
            .bind(code)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // Guest keys

    /// Saves a new key for the player and drops the oldest past [`MAX_KEYS_PER_PLAYER`].
    pub async fn pocket_key_insert(&self, code: &str, player_id: &str, key_hash: &str, now: DateTime<Utc>) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO pocket_guest_keys (key_hash, pocket_code, player_id, created_at) VALUES (?, ?, ?, ?)")
            .bind(key_hash)
            .bind(code)
            .bind(player_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM pocket_guest_keys WHERE pocket_code = ? AND player_id = ? AND key_hash NOT IN \
             (SELECT key_hash FROM pocket_guest_keys WHERE pocket_code = ? AND player_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?)",
        )
        .bind(code)
        .bind(player_id)
        .bind(code)
        .bind(player_id)
        .bind(MAX_KEYS_PER_PLAYER)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The player a key belongs to, only within `code` (a key for another pocket is nobody here).
    pub async fn pocket_key_player(&self, code: &str, key_hash: &str) -> Result<Option<PlayerRow>, DbError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {} FROM pocket_players p JOIN pocket_guest_keys k ON k.pocket_code = p.pocket_code AND k.player_id = p.id \
             WHERE k.key_hash = ? AND k.pocket_code = ? AND p.removed_at IS NULL",
            PLAYER_COLS.split(", ").map(|c| format!("p.{c}")).collect::<Vec<_>>().join(", ")
        ))
        .bind(key_hash)
        .bind(code)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Rows a pocket still has in each table, for tests and the sweeper's checks.
    pub async fn pocket_row_counts(&self, code: &str) -> Result<[i64; 4], DbError> {
        let mut out = [0i64; 4];
        for (i, (table, col)) in [("pocket_apps", "code"), ("pocket_players", "pocket_code"), ("pocket_guest_keys", "pocket_code"), ("pocket_events", "pocket_code")]
            .iter()
            .enumerate()
        {
            let (n,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table} WHERE {col} = ?")).bind(code).fetch_one(&self.pool).await?;
            out[i] = n;
        }
        Ok(out)
    }
}
