//! Dev mode (docs/dev-mode.md): the repos each space watches (one per team, plus any the person
//! adds by hand: plan team-repos.md T1), the cards kwant writes about what landed on them and their
//! open pull requests, and the backups the production floor asks for. Kept in their own tables,
//! outside exports, like approval modes, so an imported bundle can't switch Dev mode on or off.
//! Only the person writes `dev_mode_spaces` and `dev_repos` (the routes refuse agent tokens).
//! Add-only except for one rebuild of `dev_cards` (T2), so a 0.7.1 build still reads this database.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// Most model calls Dev mode makes per space per day (plan D8).
pub const CALLS_CAP: i64 = 30;

/// Most repos one space watches (team-repos.md T4).
pub const MAX_REPOS: usize = 10;

/// Who added a `dev_repos` row: a team's repo link or folder (removed when the team stops
/// pointing at it), the person (removed only by them), or the 0.7.1 migration (decided at the
/// first reconcile; read as `hand` until then).
pub const BY_TEAM: &str = "team";
pub const BY_HAND: &str = "hand";
pub const BY_MIGRATION: &str = "migrated";
/// `dev_repos.provider` of a repo read with git alone (any-git.md G1): a non-GitHub host or a
/// team folder. Its branch is kept in `git_branch` and `branch` stays `''`, so 0.7.2 (which polls
/// only rows with a branch, through GitHub) leaves it alone.
pub const GIT: &str = "git";

/// Card kinds and statuses, as stored.
pub const LANDED: &str = "landed";
pub const REVIEW: &str = "review";
pub const RELEASE_NOTES: &str = "release_notes";
pub const PENDING: &str = "pending";
pub const READY: &str = "ready";
pub const FAILED: &str = "failed";
pub const STALE: &str = "stale";

/// "Some space has Dev mode on", kept in memory so the production floor skips its reads while
/// Dev mode is off everywhere. Read at open and re-read after every write to `dev_mode_spaces`,
/// under `writes`, so two toggles can't leave it stale. A failed read counts as on (the floor runs).
#[derive(Debug, Default)]
pub struct DevOn {
    on: std::sync::atomic::AtomicBool,
    writes: tokio::sync::Mutex<()>,
}

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS dev_mode_spaces (
        space_id TEXT PRIMARY KEY,
        on_ INTEGER NOT NULL DEFAULT 0,
        provider TEXT NOT NULL DEFAULT 'github',
        repo TEXT NOT NULL,
        branch TEXT NOT NULL,
        deploy_hint TEXT,
        backup_command TEXT,
        last_seen_sha TEXT,
        polled_at TIMESTAMP,
        poll_error TEXT,
        seen_at TIMESTAMP,
        updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
    )",
    CARDS_TABLE,
    // The repos a space watches (T1). `key` is `host/owner/name` lowercased; `team` is the team's id
    "CREATE TABLE IF NOT EXISTS dev_repos (
        space_id TEXT NOT NULL,
        key TEXT NOT NULL,
        team TEXT,
        provider TEXT NOT NULL DEFAULT 'github',
        repo TEXT NOT NULL,
        host TEXT,
        branch TEXT NOT NULL DEFAULT '',
        deploy_hint TEXT,
        last_seen_sha TEXT,
        polled_at TIMESTAMP,
        poll_error TEXT,
        added_by TEXT NOT NULL,
        added_at TIMESTAMP NOT NULL,
        PRIMARY KEY (space_id, key)
    )",
    "CREATE TABLE IF NOT EXISTS dev_backups (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        kwant_id TEXT,
        path TEXT,
        bytes INTEGER,
        ok INTEGER NOT NULL,
        error TEXT,
        taken_at TIMESTAMP NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_dev_backups_space ON dev_backups(space_id, taken_at)",
    // Model calls per space per UTC day, for the D8 cap (not in exports either)
    "CREATE TABLE IF NOT EXISTS dev_model_calls (
        space_id TEXT NOT NULL,
        day TEXT NOT NULL,
        calls INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (space_id, day)
    )",
    // Connections the person marked production (D10): `mcp:<server id>` or `composio:<toolkit slug>`
    "CREATE TABLE IF NOT EXISTS dev_prod_connections (
        space_id TEXT NOT NULL,
        connection_id TEXT NOT NULL,
        marked_at TIMESTAMP NOT NULL,
        PRIMARY KEY (space_id, connection_id)
    )",
];

/// `dev_cards` as 0.7.2 makes it: a card belongs to one repo, so PR #12 in two repos is two cards
/// (T2). Older databases are rebuilt into this shape once, by [`Database::migrate_dev_mode`].
const CARDS_TABLE: &str = "CREATE TABLE IF NOT EXISTS dev_cards (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        repo TEXT NOT NULL DEFAULT '',
        kind TEXT NOT NULL,
        ref TEXT NOT NULL,
        head_sha TEXT NOT NULL,
        status TEXT NOT NULL,
        facts TEXT NOT NULL,
        card TEXT,
        created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        posted TEXT,
        post_error TEXT,
        rev INTEGER NOT NULL DEFAULT 0,
        UNIQUE(space_id, repo, kind, ref, head_sha)
    )";

/// Downgrade guard (Ada's condition 1 for T2). A 0.7.1 build on this database watches one repo
/// (`dev_mode_spaces.repo`) and still finds cards by PR number across the whole space: its poll
/// would mark every other repo's review cards stale (their PRs aren't open in its repo) and its CI
/// recheck would read other repos' commits on its repo and overwrite their facts. Every 0.7.2
/// write to `dev_cards` bumps `rev`; a write that leaves `rev` alone comes from 0.7.1, and when it
/// would mark stale, or rewrite the facts of, a card of another repo than the one 0.7.1 watches,
/// that row is skipped (`RAISE(IGNORE)`: no error, the rest of the statement goes on). 0.7.1 keeps
/// working for its own repo. Its card inserts fail anyway (see `migrate_team_repos`).
const CARDS_071_GUARD: &str = "CREATE TRIGGER IF NOT EXISTS dev_cards_071_keeps_to_its_repo BEFORE UPDATE ON dev_cards
    WHEN NEW.rev = OLD.rev
      AND (NEW.facts IS NOT OLD.facts OR (NEW.status = 'stale' AND OLD.status != 'stale'))
      AND OLD.repo != COALESCE(
        (SELECT lower('github.com/' || s.repo) FROM dev_mode_spaces s WHERE s.space_id = OLD.space_id AND trim(s.repo) != ''), '')
    BEGIN SELECT RAISE(IGNORE); END";

const CARD_INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_dev_cards_space ON dev_cards(space_id, kind, created_at)",
    "CREATE INDEX IF NOT EXISTS idx_dev_cards_repo ON dev_cards(space_id, repo, kind, created_at)",
];

/// One watched repo of a space (T1) and its poll state.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct DevRepoRow {
    pub space_id: String,
    pub key: String,
    /// The id of the team it came from (`team` rows), or of the team that points at a hand row.
    pub team: Option<String>,
    pub provider: String,
    /// `owner/name`.
    pub repo: String,
    pub host: Option<String>,
    pub branch: String,
    pub deploy_hint: Option<String>,
    pub last_seen_sha: Option<String>,
    pub polled_at: Option<DateTime<Utc>>,
    pub poll_error: Option<String>,
    pub added_by: String,
    pub added_at: DateTime<Utc>,
    /// A [`GIT`] row's fetch URL (userinfo stripped) or team folder: kwant's own, never returned.
    #[sqlx(default)]
    pub git_url: Option<String>,
}

impl DevRepoRow {
    /// `team` or `hand` (a migrated row reads as `hand` until reconcile decides).
    pub fn added_by_shown(&self) -> &'static str {
        if self.added_by == BY_TEAM {
            BY_TEAM
        } else {
            BY_HAND
        }
    }

    /// Read with git alone (no GitHub connection behind it).
    pub fn is_git(&self) -> bool {
        self.provider == GIT
    }
}

/// `dev_repos` columns as kwant reads them: a [`GIT`] row's branch is its `git_branch`.
const REPO_COLS: &str = "space_id, key, team, provider, repo, host, CASE WHEN provider = 'git' THEN COALESCE(git_branch, '') ELSE branch END AS branch, deploy_hint, last_seen_sha, polled_at, poll_error, added_by, added_at, git_url";

/// The branch a row watches, in SQL (see [`REPO_COLS`]).
const ROW_BRANCH: &str = "(CASE WHEN provider = 'git' THEN COALESCE(git_branch, '') ELSE branch END)";

/// What adding a repo row did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoAdd {
    Added,
    /// A row with that key was there already (its team label may have been updated).
    Kept,
    /// The space already watches [`MAX_REPOS`].
    TooMany,
}

/// The repo key of `owner/name` on `host`: `host/owner/name` lowercased.
pub fn repo_key(host: &str, repo: &str) -> String {
    format!("{host}/{repo}").to_lowercase()
}

/// A space's Dev mode settings. Since 0.7.2 its `repo`, `branch` and poll columns are 0.7.1's,
/// kept for a downgrade; the watched repos are [`DevRepoRow`]s. `branch` is still written when
/// Dev mode turns on with no repo (a team folder's branch, for the production floor).
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct DevModeRow {
    pub space_id: String,
    #[sqlx(rename = "on_")]
    pub on: bool,
    pub provider: String,
    pub repo: String,
    pub branch: String,
    pub deploy_hint: Option<String>,
    pub backup_command: Option<String>,
    pub last_seen_sha: Option<String>,
    pub polled_at: Option<DateTime<Utc>>,
    pub poll_error: Option<String>,
    /// When the person last looked at what landed ("Mark seen"): the desk's rollup counts cards
    /// after it.
    pub seen_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    /// When the person last opened the desk (flow F6): the row badge counts review cards after it.
    /// Its own column since 0.7.2, so opening the desk no longer empties the landed rollup.
    #[sqlx(default)]
    pub reviews_seen_at: Option<DateTime<Utc>>,
}

/// One card: what landed, a review of an open PR head, or a release-notes draft.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct DevCardRow {
    pub id: String,
    pub space_id: String,
    pub kind: String,
    #[sqlx(rename = "ref")]
    pub ref_: String,
    pub head_sha: String,
    pub status: String,
    /// `server::dev::facts::DevFacts` JSON (or `ReleaseFacts` for release notes).
    pub facts: String,
    /// The card JSON (plan §3.4) once generated.
    pub card: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The repo key the card is about (T2); `""` for a card a 0.7.1 space made with no repo.
    #[sqlx(default)]
    pub repo: String,
    /// Review cards: `{at, login, url, comments}` once posted to GitHub (post-to-github.md P1).
    /// A column of its own, so a card regenerated after CI keeps it.
    #[sqlx(default)]
    pub posted: Option<String>,
    /// Review cards: `{kind, message}` from the last post that didn't go through.
    #[sqlx(default)]
    pub post_error: Option<String>,
}

#[derive(Debug, Clone, FromRow, PartialEq, serde::Serialize)]
pub struct DevBackupRow {
    pub id: String,
    pub space_id: String,
    pub kwant_id: Option<String>,
    pub path: Option<String>,
    pub bytes: Option<i64>,
    pub ok: bool,
    pub error: Option<String>,
    pub taken_at: DateTime<Utc>,
}

/// Adds `dev_mode_spaces.reviews_seen_at` and fills it from `seen_at` (run once, see
/// `migrate_dev_mode`). Additive only: older builds keep reading and writing the table.
const MIGRATE_REVIEWS_SEEN: [&str; 2] = [
    "ALTER TABLE dev_mode_spaces ADD COLUMN reviews_seen_at TIMESTAMP",
    "UPDATE dev_mode_spaces SET reviews_seen_at = seen_at WHERE reviews_seen_at IS NULL",
];

const CARD_COLS: &str = "id, space_id, repo, kind, ref, head_sha, status, facts, card, created_at, updated_at, posted, post_error";

impl Database {
    pub(super) async fn migrate_dev_mode(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        // Posting a review to GitHub came in 0.7.1; older builds name their columns, so they
        // still read this table
        for column in ["posted", "post_error"] {
            if !self.dev_cards_has(column).await? {
                sqlx::query(&format!("ALTER TABLE dev_cards ADD COLUMN {column} TEXT")).execute(&self.pool).await?;
            }
        }
        // 0.7.2: the badge's "seen" (desk opened) apart from the rollup's (Mark seen). Added once,
        // with the backfill in the same transaction, so a database from before keeps its badge
        // state and a later start never copies a Mark seen into the badge.
        let has: Option<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('dev_mode_spaces') WHERE name = 'reviews_seen_at'")
            .fetch_optional(&self.pool)
            .await?;
        if has.is_none() {
            let mut tx = self.pool.begin().await?;
            sqlx::query(MIGRATE_REVIEWS_SEEN[0]).execute(&mut *tx).await?;
            sqlx::query(MIGRATE_REVIEWS_SEEN[1]).execute(&mut *tx).await?;
            tx.commit().await?;
        }
        if !self.dev_cards_has("repo").await? {
            self.migrate_team_repos().await?;
        }
        // Built before the guard came in (0.7.2 development databases only)
        if !self.dev_cards_has("rev").await? {
            sqlx::query("ALTER TABLE dev_cards ADD COLUMN rev INTEGER NOT NULL DEFAULT 0").execute(&self.pool).await?;
        }
        sqlx::query(CARDS_071_GUARD).execute(&self.pool).await?;
        // 0.7.3: repos read with git alone (any-git.md G1). Additive; 0.7.2 never names them
        for column in ["git_branch", "git_url"] {
            let has: Option<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('dev_repos') WHERE name = ?").bind(column).fetch_optional(&self.pool).await?;
            if has.is_none() {
                sqlx::query(&format!("ALTER TABLE dev_repos ADD COLUMN {column} TEXT")).execute(&self.pool).await?;
            }
        }
        for sql in CARD_INDEXES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        // 0.7.6: briefs of Jira tickets (dev_ticket_briefs.rs)
        self.migrate_ticket_briefs().await?;
        // 0.8.0: pre-push cards (B2, dev_prepush.rs)
        self.migrate_prepush().await?;
        // 0.9.0: CI and review comments back to the author bot (dev_fix_it.rs)
        self.migrate_fix_it().await?;
        Ok(())
    }

    async fn dev_cards_has(&self, column: &str) -> Result<bool, DbError> {
        let has: Option<(String,)> =
            sqlx::query_as("SELECT name FROM pragma_table_info('dev_cards') WHERE name = ?").bind(column).fetch_optional(&self.pool).await?;
        Ok(has.is_some())
    }

    /// 0.7.1 → 0.7.2 (team-repos.md T2), once, in one transaction: each space's one repo becomes
    /// a `dev_repos` row with its branch, last seen commit and poll state (so nothing is announced
    /// again), and `dev_cards` is rebuilt with a `repo` column in its unique key. A card's repo is
    /// the one its facts name, else its space's. 0.7.1 kept `owner/name` without a host, so the
    /// key is github.com's; reconcile re-keys it if a team's source is on a GitHub Enterprise host.
    ///
    /// Going back to 0.7.1 afterwards is safe but limited, by design: 0.7.1 still reads every card
    /// and repo setting, but its card insert names the old unique key (`ON CONFLICT(space_id, kind,
    /// ref, head_sha)`), which this table no longer has, so SQLite refuses it; 0.7.1 logs "could not
    /// save a card" and its poll goes on. It never sees `dev_repos`, so coming back up to 0.7.2
    /// picks each repo up where 0.7.2 left it (`dev_repos.last_seen_sha`) and makes the cards for
    /// what landed in between, once; this function doesn't run again (`dev_cards` has `repo`).
    /// [`CARDS_071_GUARD`] keeps 0.7.1 off other repos' cards.
    async fn migrate_team_repos(&self) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT OR IGNORE INTO dev_repos (space_id, key, team, provider, repo, host, branch, deploy_hint, last_seen_sha, polled_at, poll_error, added_by, added_at)
             SELECT space_id, lower('github.com/' || repo), NULL, provider, repo, 'github.com', branch, deploy_hint, last_seen_sha, polled_at, poll_error, ?1, updated_at
             FROM dev_mode_spaces WHERE trim(repo) != ''",
        )
        .bind(BY_MIGRATION)
        .execute(&mut *tx)
        .await?;
        sqlx::query(&CARDS_TABLE.replace("IF NOT EXISTS dev_cards", "dev_cards_v2")).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO dev_cards_v2 (id, space_id, repo, kind, ref, head_sha, status, facts, card, created_at, updated_at, posted, post_error)
             SELECT c.id, c.space_id,
               COALESCE(
                 (SELECT lower('github.com/' || r) FROM (SELECT CASE WHEN json_valid(c.facts) THEN json_extract(c.facts, '$.repo') END AS r)
                  WHERE typeof(r) = 'text' AND instr(r, '/') > 0),
                 (SELECT lower('github.com/' || s.repo) FROM dev_mode_spaces s WHERE s.space_id = c.space_id AND trim(s.repo) != ''),
                 ''),
               c.kind, c.ref, c.head_sha, c.status, c.facts, c.card, c.created_at, c.updated_at, c.posted, c.post_error
             FROM dev_cards c",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("DROP TABLE dev_cards").execute(&mut *tx).await?;
        sqlx::query("ALTER TABLE dev_cards_v2 RENAME TO dev_cards").execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// False only when no space has Dev mode on: then there is no production floor anywhere.
    pub fn any_dev_mode_on(&self) -> bool {
        self.dev_on.on.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(super) async fn refresh_dev_on(&self) {
        let on = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM dev_mode_spaces WHERE on_ = 1)").fetch_one(&self.pool).await;
        self.dev_on.on.store(on.unwrap_or(true), std::sync::atomic::Ordering::SeqCst);
    }

    pub async fn dev_mode(&self, space_id: &str) -> Result<Option<DevModeRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM dev_mode_spaces WHERE space_id = ?").bind(space_id).fetch_optional(&self.pool).await?)
    }

    /// Every space with Dev mode on, for the poller.
    pub async fn dev_mode_on_spaces(&self) -> Result<Vec<DevModeRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM dev_mode_spaces WHERE on_ = 1 ORDER BY space_id").fetch_all(&self.pool).await?)
    }

    /// Saves the person's settings: the switch and the backup command. A new row gets no 0.7.1
    /// repo; `branch` is what the production floor treats as the main branch while the space
    /// watches no repo (a team folder's), and an existing row's 0.7.1 columns stay as they were.
    pub async fn set_dev_mode(&self, space_id: &str, on: bool, branch: Option<&str>, backup_command: Option<&str>) -> Result<DevModeRow, DbError> {
        let _writes = self.dev_on.writes.lock().await;
        if on {
            // Before the row, so no call slips between the write and the flag
            self.dev_on.on.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let done = sqlx::query(
            "INSERT INTO dev_mode_spaces (space_id, on_, repo, branch, backup_command, updated_at) VALUES (?1, ?2, '', COALESCE(?3, ''), ?4, ?5)
             ON CONFLICT(space_id) DO UPDATE SET on_ = excluded.on_, backup_command = excluded.backup_command, updated_at = excluded.updated_at,
               branch = CASE WHEN ?3 IS NOT NULL AND dev_mode_spaces.repo = '' THEN ?3 ELSE dev_mode_spaces.branch END",
        )
        .bind(space_id)
        .bind(on)
        .bind(branch)
        .bind(backup_command)
        .bind(Utc::now())
        .execute(&self.pool)
        .await;
        self.refresh_dev_on().await;
        done?;
        Ok(self.dev_mode(space_id).await?.expect("just written"))
    }

    /// [`Self::set_dev_mode`] plus one hand-added repo on github.com (the 0.7.1 shape, for tests
    /// and callers that name one repo). An empty `repo` adds none.
    pub async fn put_dev_mode(
        &self,
        space_id: &str,
        on: bool,
        repo: &str,
        branch: &str,
        deploy_hint: Option<&str>,
        backup_command: Option<&str>,
    ) -> Result<DevModeRow, DbError> {
        let row = self.set_dev_mode(space_id, on, Some(branch), backup_command).await?;
        if !repo.trim().is_empty() {
            let key = repo_key("github.com", repo);
            self.add_dev_repo(space_id, &key, None, repo, Some("github.com"), branch, deploy_hint, BY_HAND).await?;
            self.set_dev_repo_branch(space_id, &key, branch).await?;
        }
        Ok(row)
    }

    /// A space's watched repos, oldest first.
    pub async fn dev_repos(&self, space_id: &str) -> Result<Vec<DevRepoRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {REPO_COLS} FROM dev_repos WHERE space_id = ? ORDER BY added_at, key")).bind(space_id).fetch_all(&self.pool).await?)
    }

    pub async fn dev_repo(&self, space_id: &str, key: &str) -> Result<Option<DevRepoRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {REPO_COLS} FROM dev_repos WHERE space_id = ? AND key = ?")).bind(space_id).bind(key).fetch_optional(&self.pool).await?)
    }

    /// Adds a watched repo, unless the space watches [`MAX_REPOS`] already (T4). A row with the
    /// same key is kept as it is (its poll state too), except that a team source names its team
    /// on it, and a migrated row a team points at becomes that team's.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_dev_repo(
        &self,
        space_id: &str,
        key: &str,
        team: Option<&str>,
        repo: &str,
        host: Option<&str>,
        branch: &str,
        deploy_hint: Option<&str>,
        added_by: &str,
    ) -> Result<RepoAdd, DbError> {
        self.add_repo_row(space_id, key, team, repo, host, branch, deploy_hint, added_by, None).await
    }

    /// [`Self::add_dev_repo`] for a repo read with git alone ([`GIT`]): `url` is what git fetches
    /// from (userinfo stripped by the caller) or the team folder, `branch` may be `''` (the
    /// poller takes the default branch).
    #[allow(clippy::too_many_arguments)]
    pub async fn add_dev_repo_git(&self, space_id: &str, key: &str, team: Option<&str>, repo: &str, host: Option<&str>, branch: &str, url: &str, added_by: &str) -> Result<RepoAdd, DbError> {
        self.add_repo_row(space_id, key, team, repo, host, branch, None, added_by, Some(url)).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn add_repo_row(
        &self,
        space_id: &str,
        key: &str,
        team: Option<&str>,
        repo: &str,
        host: Option<&str>,
        branch: &str,
        deploy_hint: Option<&str>,
        added_by: &str,
        git_url: Option<&str>,
    ) -> Result<RepoAdd, DbError> {
        let _writes = self.dev_on.writes.lock().await;
        // Reads, then writes: holds the write lock from BEGIN, or a poller's write in between
        // fails the insert at once with "database is locked"
        let mut tx = self.begin_write().await?;
        let have: Option<(String,)> = sqlx::query_as("SELECT added_by FROM dev_repos WHERE space_id = ? AND key = ?").bind(space_id).bind(key).fetch_optional(&mut *tx).await?;
        if let Some((by,)) = have {
            if team.is_some() {
                let by = if by == BY_MIGRATION && added_by == BY_TEAM { BY_TEAM.to_string() } else { by };
                sqlx::query("UPDATE dev_repos SET team = ?3, added_by = ?4 WHERE space_id = ?1 AND key = ?2")
                    .bind(space_id)
                    .bind(key)
                    .bind(team)
                    .bind(by)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
            return Ok(RepoAdd::Kept);
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dev_repos WHERE space_id = ?").bind(space_id).fetch_one(&mut *tx).await?;
        if count >= MAX_REPOS as i64 {
            tx.rollback().await?;
            return Ok(RepoAdd::TooMany);
        }
        // A git row keeps its branch in git_branch; `branch` stays '' for 0.7.2's poller
        let (provider, gh_branch, git_branch) = match git_url {
            Some(_) => (GIT, "", Some(branch)),
            None => ("github", branch, None),
        };
        sqlx::query(
            "INSERT INTO dev_repos (space_id, key, team, provider, repo, host, branch, deploy_hint, added_by, added_at, git_branch, git_url) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(space_id)
        .bind(key)
        .bind(team)
        .bind(provider)
        .bind(repo)
        .bind(host)
        .bind(gh_branch)
        .bind(deploy_hint)
        .bind(added_by)
        .bind(Utc::now())
        .bind(git_branch)
        .bind(git_url)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(RepoAdd::Added)
    }

    /// Sets a row's branch; a new branch starts its watch over (last seen commit and poll state
    /// cleared), the same one keeps them. False: no such row.
    pub async fn set_dev_repo_branch(&self, space_id: &str, key: &str, branch: &str) -> Result<bool, DbError> {
        let done = sqlx::query(
            &format!(
                "UPDATE dev_repos SET
                   last_seen_sha = CASE WHEN {ROW_BRANCH} = ?3 THEN last_seen_sha ELSE NULL END,
                   polled_at = CASE WHEN {ROW_BRANCH} = ?3 THEN polled_at ELSE NULL END,
                   poll_error = CASE WHEN {ROW_BRANCH} = ?3 THEN poll_error ELSE NULL END,
                   branch = CASE WHEN provider = 'git' THEN '' ELSE ?3 END,
                   git_branch = CASE WHEN provider = 'git' THEN ?3 ELSE git_branch END
                 WHERE space_id = ?1 AND key = ?2"
            ),
        )
        .bind(space_id)
        .bind(key)
        .bind(branch)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Records why a row can't be read without marking it polled (a row added while off whose
    /// repo the switch-on check didn't find).
    pub async fn set_dev_repo_error(&self, space_id: &str, key: &str, error: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE dev_repos SET poll_error = ?3 WHERE space_id = ?1 AND key = ?2").bind(space_id).bind(key).bind(error).execute(&self.pool).await?;
        Ok(done.rows_affected() == 1)
    }

    /// Sets a row's deploy hint (from its root files, when GitHub was read).
    pub async fn set_dev_repo_hint(&self, space_id: &str, key: &str, hint: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_repos SET deploy_hint = ?3 WHERE space_id = ?1 AND key = ?2").bind(space_id).bind(key).bind(hint).execute(&self.pool).await?;
        Ok(())
    }

    /// Sets who a row belongs to: `team` (with its team) or `hand`.
    pub async fn set_dev_repo_owner(&self, space_id: &str, key: &str, added_by: &str, team: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_repos SET added_by = ?3, team = ?4 WHERE space_id = ?1 AND key = ?2").bind(space_id).bind(key).bind(added_by).bind(team).execute(&self.pool).await?;
        Ok(())
    }

    /// Removes a watched repo. Its cards stay, with their repo. False: no such row.
    pub async fn delete_dev_repo(&self, space_id: &str, key: &str) -> Result<bool, DbError> {
        let done = sqlx::query("DELETE FROM dev_repos WHERE space_id = ? AND key = ?").bind(space_id).bind(key).execute(&self.pool).await?;
        Ok(done.rows_affected() == 1)
    }

    /// Moves a row and its cards to another key (a migrated github.com key whose repo is on a
    /// GitHub Enterprise host). False when the new key is taken or the old one is gone.
    pub async fn rekey_dev_repo(&self, space_id: &str, from: &str, to: &str, host: &str) -> Result<bool, DbError> {
        let mut tx = self.begin_write().await?;
        let taken: Option<(String,)> = sqlx::query_as("SELECT key FROM dev_repos WHERE space_id = ? AND key = ?").bind(space_id).bind(to).fetch_optional(&mut *tx).await?;
        if taken.is_some() {
            tx.rollback().await?;
            return Ok(false);
        }
        let done = sqlx::query("UPDATE dev_repos SET key = ?3, host = ?4 WHERE space_id = ?1 AND key = ?2").bind(space_id).bind(from).bind(to).bind(host).execute(&mut *tx).await?;
        if done.rows_affected() != 1 {
            return Ok(false);
        }
        sqlx::query("UPDATE OR IGNORE dev_cards SET repo = ?3, rev = rev + 1 WHERE space_id = ?1 AND repo = ?2").bind(space_id).bind(from).bind(to).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Records one poll of one repo: the newest merge it turned into a card (`None` keeps the
    /// one before), and its error (`None`: it worked).
    pub async fn dev_repo_polled(&self, space_id: &str, key: &str, last_seen_sha: Option<&str>, error: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_repos SET last_seen_sha = COALESCE(?3, last_seen_sha), polled_at = ?4, poll_error = ?5 WHERE space_id = ?1 AND key = ?2")
            .bind(space_id)
            .bind(key)
            .bind(last_seen_sha)
            .bind(Utc::now())
            .bind(error)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Sets a team's repo link (`PUT dev_mode {repo, team}`, T8). The caller strips userinfo.
    pub async fn set_team_repo_url(&self, team_id: &str, repo_url: Option<&str>) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE teams SET repo_url = ? WHERE id = ?").bind(repo_url).bind(team_id).execute(&self.pool).await?;
        Ok(done.rows_affected() == 1)
    }

    /// "Mark seen" on what landed: the rollup starts over from `at`. The badge is left alone.
    pub async fn dev_mode_seen(&self, space_id: &str, at: DateTime<Utc>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_mode_spaces SET seen_at = ?2 WHERE space_id = ?1").bind(space_id).bind(at).execute(&self.pool).await?;
        Ok(())
    }

    /// The desk was opened (flow F6): the review badge starts over from `at`. The rollup is left
    /// alone, so what landed since the last Mark seen still shows.
    pub async fn dev_mode_reviews_seen(&self, space_id: &str, at: DateTime<Utc>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_mode_spaces SET reviews_seen_at = ?2 WHERE space_id = ?1").bind(space_id).bind(at).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete_dev_mode_of_space(&self, space_id: &str) -> Result<(), DbError> {
        let _writes = self.dev_on.writes.lock().await;
        let done = self.delete_dev_rows(space_id).await;
        self.refresh_dev_on().await;
        done
    }

    async fn delete_dev_rows(&self, space_id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let tables = ["dev_mode_spaces", "dev_repos", "dev_cards", "dev_backups", "dev_model_calls", "dev_prod_connections"];
        for t in tables.into_iter().chain(super::dev_ticket_briefs::BRIEF_TABLES).chain(super::dev_prepush::PREPUSH_TABLES).chain(super::dev_fix_it::FIX_TABLES) {
            sqlx::query(&format!("DELETE FROM {t} WHERE space_id = ?")).bind(space_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The connection ids the person marked production in a space (D10).
    pub async fn prod_connections(&self, space_id: &str) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT connection_id FROM dev_prod_connections WHERE space_id = ? ORDER BY connection_id")
            .bind(space_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Marks or unmarks a connection production.
    pub async fn set_prod_connection(&self, space_id: &str, connection_id: &str, production: bool) -> Result<(), DbError> {
        if production {
            sqlx::query("INSERT INTO dev_prod_connections (space_id, connection_id, marked_at) VALUES (?, ?, ?) ON CONFLICT DO NOTHING")
                .bind(space_id)
                .bind(connection_id)
                .bind(Utc::now())
                .execute(&self.pool)
                .await?;
        } else {
            sqlx::query("DELETE FROM dev_prod_connections WHERE space_id = ? AND connection_id = ?").bind(space_id).bind(connection_id).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Adds a pending card with no repo (release notes before 0.7.2, tests). See [`Self::insert_dev_card_in`].
    pub async fn insert_dev_card(&self, space_id: &str, kind: &str, ref_: &str, head_sha: &str, facts: &str) -> Result<Option<DevCardRow>, DbError> {
        self.insert_dev_card_in(space_id, "", kind, ref_, head_sha, facts).await
    }

    /// Adds a pending card about repo `repo` (its key). `None` when one with the same space,
    /// repo, kind, ref and head already exists.
    pub async fn insert_dev_card_in(&self, space_id: &str, repo: &str, kind: &str, ref_: &str, head_sha: &str, facts: &str) -> Result<Option<DevCardRow>, DbError> {
        let id = format!("dvc_{}", uuid::Uuid::new_v4().simple());
        let now = Utc::now();
        let done = sqlx::query(
            "INSERT INTO dev_cards (id, space_id, repo, kind, ref, head_sha, status, facts, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?) ON CONFLICT(space_id, repo, kind, ref, head_sha) DO NOTHING",
        )
        .bind(&id)
        .bind(space_id)
        .bind(repo)
        .bind(kind)
        .bind(ref_)
        .bind(head_sha)
        .bind(facts)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(None);
        }
        self.dev_card(&id).await
    }

    pub async fn dev_card(&self, id: &str) -> Result<Option<DevCardRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {CARD_COLS} FROM dev_cards WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?)
    }

    /// Sets a card's status, and its card JSON when given (`None` keeps the stored one). A `stale`
    /// card stays as it is: a review that finishes after its PR merged (or moved on) must not come
    /// back to life. `None` when nothing was written (no such card, or it is stale).
    pub async fn set_dev_card(&self, id: &str, status: &str, card: Option<&str>) -> Result<Option<DevCardRow>, DbError> {
        let done = sqlx::query("UPDATE dev_cards SET status = ?2, card = COALESCE(?3, card), updated_at = ?4, rev = rev + 1 WHERE id = ?1 AND status != 'stale'")
            .bind(id)
            .bind(status)
            .bind(card)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Ok(None);
        }
        self.dev_card(id).await
    }

    /// New facts for a card (CI finished after the card was made): back to pending so `generate`
    /// runs again; the card JSON stays until then.
    pub async fn set_dev_card_facts(&self, id: &str, facts: &str) -> Result<Option<DevCardRow>, DbError> {
        sqlx::query("UPDATE dev_cards SET facts = ?2, status = 'pending', updated_at = ?3, rev = rev + 1 WHERE id = ?1")
            .bind(id)
            .bind(facts)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        self.dev_card(id).await
    }

    /// Replaces a card's JSON only while it still has `expect` (compare-and-set), so two finding
    /// actions can't overwrite each other. False when it changed in between.
    pub async fn swap_dev_card_json(&self, id: &str, expect: Option<&str>, card: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE dev_cards SET card = ?3, updated_at = ?4, rev = rev + 1 WHERE id = ?1 AND card IS ?2")
            .bind(id)
            .bind(expect)
            .bind(card)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Records a review as posted, only while none is (P6): false when another post got there
    /// first. Clears `post_error`.
    pub async fn set_dev_card_posted(&self, id: &str, posted: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE dev_cards SET posted = ?2, post_error = NULL, updated_at = ?3, rev = rev + 1 WHERE id = ?1 AND posted IS NULL")
            .bind(id)
            .bind(posted)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Records why a post didn't go through (`None` clears it). A posted card keeps no error.
    pub async fn set_dev_card_post_error(&self, id: &str, error: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_cards SET post_error = ?2, updated_at = ?3, rev = rev + 1 WHERE id = ?1 AND posted IS NULL")
            .bind(id)
            .bind(error)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// A space's cards of one kind, every repo's, newest first, created before `before` when given.
    pub async fn dev_cards(&self, space_id: &str, kind: &str, before: Option<DateTime<Utc>>, limit: i64) -> Result<Vec<DevCardRow>, DbError> {
        self.dev_cards_in(space_id, None, kind, before, limit).await
    }

    /// [`Self::dev_cards`] of one repo (its key) when `repo` is given.
    pub async fn dev_cards_in(&self, space_id: &str, repo: Option<&str>, kind: &str, before: Option<DateTime<Utc>>, limit: i64) -> Result<Vec<DevCardRow>, DbError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {CARD_COLS} FROM dev_cards WHERE space_id = ?1 AND kind = ?2 AND (?3 IS NULL OR created_at < ?3) AND (?5 IS NULL OR repo = ?5)
             ORDER BY created_at DESC, id DESC LIMIT ?4"
        ))
        .bind(space_id)
        .bind(kind)
        .bind(before)
        .bind(limit)
        .bind(repo)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Cards of one kind and ref (every head a PR had) in any repo, newest first.
    pub async fn dev_cards_of_ref(&self, space_id: &str, kind: &str, ref_: &str) -> Result<Vec<DevCardRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {CARD_COLS} FROM dev_cards WHERE space_id = ? AND kind = ? AND ref = ? ORDER BY created_at DESC, id DESC"))
            .bind(space_id)
            .bind(kind)
            .bind(ref_)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Cards of one repo, kind and ref, newest first: PR #12 of one repo isn't another's.
    pub async fn dev_cards_of_ref_in(&self, space_id: &str, repo: &str, kind: &str, ref_: &str) -> Result<Vec<DevCardRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {CARD_COLS} FROM dev_cards WHERE space_id = ? AND repo = ? AND kind = ? AND ref = ? ORDER BY created_at DESC, id DESC"))
            .bind(space_id)
            .bind(repo)
            .bind(kind)
            .bind(ref_)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Cards still pending (a restart left them), oldest first.
    pub async fn pending_dev_cards(&self) -> Result<Vec<DevCardRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {CARD_COLS} FROM dev_cards WHERE status = 'pending' ORDER BY created_at")).fetch_all(&self.pool).await?)
    }

    /// Marks every review card of a PR (in any repo) whose head isn't `keep_head` stale; returns
    /// their ids. The poller uses [`Self::stale_dev_reviews_in`].
    pub async fn stale_dev_reviews(&self, space_id: &str, ref_: &str, keep_head: Option<&str>) -> Result<Vec<String>, DbError> {
        self.stale_reviews(space_id, None, ref_, keep_head).await
    }

    /// Marks every review card of one repo's PR whose head isn't `keep_head` stale.
    pub async fn stale_dev_reviews_in(&self, space_id: &str, repo: &str, ref_: &str, keep_head: Option<&str>) -> Result<Vec<String>, DbError> {
        self.stale_reviews(space_id, Some(repo), ref_, keep_head).await
    }

    async fn stale_reviews(&self, space_id: &str, repo: Option<&str>, ref_: &str, keep_head: Option<&str>) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "UPDATE dev_cards SET status = 'stale', updated_at = ?4, rev = rev + 1
             WHERE space_id = ?1 AND kind = 'review' AND ref = ?2 AND status != 'stale' AND (?3 IS NULL OR head_sha != ?3)
               AND (?5 IS NULL OR repo = ?5) RETURNING id",
        )
        .bind(space_id)
        .bind(ref_)
        .bind(keep_head)
        .bind(Utc::now())
        .bind(repo)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Model calls Dev mode made in a space today (the UTC day of `dev::now()`).
    pub async fn dev_calls_today(&self, space_id: &str) -> Result<i64, DbError> {
        self.dev_calls_on(space_id, &crate::server::dev::utc_day(crate::server::dev::now())).await
    }

    pub async fn dev_calls_on(&self, space_id: &str, day: &str) -> Result<i64, DbError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT calls FROM dev_model_calls WHERE space_id = ? AND day = ?")
            .bind(space_id)
            .bind(day)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0).unwrap_or(0))
    }

    /// Takes one model call from today's cap (`dev::call_cap()`, default [`CALLS_CAP`]): false (and
    /// nothing counted) once it's used up.
    pub async fn take_dev_call(&self, space_id: &str) -> Result<bool, DbError> {
        self.take_dev_call_at(space_id, crate::server::dev::now(), crate::server::dev::call_cap()).await
    }

    /// [`Self::take_dev_call`] on the UTC day of `now`, against `cap`.
    pub async fn take_dev_call_at(&self, space_id: &str, now: DateTime<Utc>, cap: i64) -> Result<bool, DbError> {
        if cap <= 0 {
            return Ok(false);
        }
        let done = sqlx::query(
            "INSERT INTO dev_model_calls (space_id, day, calls) VALUES (?1, ?2, 1)
             ON CONFLICT(space_id, day) DO UPDATE SET calls = calls + 1 WHERE calls < ?3",
        )
        .bind(space_id)
        .bind(crate::server::dev::utc_day(now))
        .bind(cap)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    pub async fn insert_dev_backup(&self, b: &DevBackupRow) -> Result<(), DbError> {
        sqlx::query("INSERT INTO dev_backups (id, space_id, kwant_id, path, bytes, ok, error, taken_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&b.id)
            .bind(&b.space_id)
            .bind(&b.kwant_id)
            .bind(&b.path)
            .bind(b.bytes)
            .bind(b.ok)
            .bind(&b.error)
            .bind(b.taken_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The space's newest backup, and its newest one that worked.
    pub async fn latest_dev_backups(&self, space_id: &str) -> Result<(Option<DevBackupRow>, Option<DevBackupRow>), DbError> {
        let last = sqlx::query_as("SELECT * FROM dev_backups WHERE space_id = ? ORDER BY taken_at DESC LIMIT 1").bind(space_id).fetch_optional(&self.pool).await?;
        let ok = sqlx::query_as("SELECT * FROM dev_backups WHERE space_id = ? AND ok = 1 ORDER BY taken_at DESC LIMIT 1")
            .bind(space_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok((last, ok))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACE: &str = "space_demo";

    async fn db() -> (Database, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        (Database::init(tmp.path()).await.unwrap(), tmp)
    }

    /// Puts the database back in 0.7.1's shape: `dev_cards` with the old unique key and no `repo`,
    /// no `dev_repos`, no `dev_mode_spaces.reviews_seen_at` (#357), and one space watching
    /// `Demo-Org/App` on `main` whose desk was last seen at 09:00.
    async fn as_071(db: &Database) {
        for sql in [
            "ALTER TABLE dev_mode_spaces DROP COLUMN reviews_seen_at",
            "DROP TABLE dev_repos",
            "DROP TABLE dev_cards",
            "CREATE TABLE dev_cards (
                id TEXT PRIMARY KEY, space_id TEXT NOT NULL, kind TEXT NOT NULL, ref TEXT NOT NULL,
                head_sha TEXT NOT NULL, status TEXT NOT NULL, facts TEXT NOT NULL, card TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                posted TEXT, post_error TEXT,
                UNIQUE(space_id, kind, ref, head_sha))",
            "INSERT INTO dev_mode_spaces (space_id, on_, repo, branch, deploy_hint, last_seen_sha, polled_at, poll_error, seen_at, updated_at)
             VALUES ('space_demo', 1, 'Demo-Org/App', 'main', 'fly', 'aaaa', '2026-11-06T10:00:00Z', NULL, '2026-11-06T09:00:00Z', '2026-11-05T10:00:00Z')",
            "INSERT INTO dev_mode_spaces (space_id, on_, repo, branch, updated_at) VALUES ('space_empty', 0, '', 'main', '2026-11-05T10:00:00Z')",
            "INSERT INTO dev_cards (id, space_id, kind, ref, head_sha, status, facts, card, posted)
             VALUES ('dvc_landed', 'space_demo', 'landed', 'aaaa', 'aaaa', 'ready', '{\"repo\":\"Demo-Org/App\",\"sha\":\"aaaa\"}', '{\"headline\":\"x\"}', NULL)",
            "INSERT INTO dev_cards (id, space_id, kind, ref, head_sha, status, facts, card, posted)
             VALUES ('dvc_review', 'space_demo', 'review', 'pr:12', 'bbbb', 'ready', '{\"repo\":\"demo-org/app\",\"pr\":12}', NULL, '{\"url\":\"u\"}')",
            "INSERT INTO dev_cards (id, space_id, kind, ref, head_sha, status, facts) VALUES ('dvc_notes', 'space_demo', 'release_notes', 'since:', 'x', 'ready', 'not json')",
            "INSERT INTO dev_cards (id, space_id, kind, ref, head_sha, status, facts) VALUES ('dvc_orphan', 'space_gone', 'landed', 'cccc', 'cccc', 'ready', '{}')",
        ] {
            sqlx::query(sql).execute(&db.pool).await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_071_database_migrates_once_and_nothing_is_announced_again() {
        let (db, _tmp) = db().await;
        as_071(&db).await;
        db.migrate_dev_mode().await.unwrap();

        // The space's one repo is a row with its poll state, keyed on github.com
        let rows = db.dev_repos(SPACE).await.unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!((r.key.as_str(), r.repo.as_str(), r.host.as_deref(), r.branch.as_str()), ("github.com/demo-org/app", "Demo-Org/App", Some("github.com"), "main"));
        assert_eq!((r.last_seen_sha.as_deref(), r.deploy_hint.as_deref(), r.added_by.as_str(), r.team.as_deref()), (Some("aaaa"), Some("fly"), BY_MIGRATION, None));
        assert_eq!(r.added_by_shown(), BY_HAND, "a migrated row reads as hand until reconcile");
        assert!(r.polled_at.is_some());
        // A space with no repo gets no row; its settings stay
        assert!(db.dev_repos("space_empty").await.unwrap().is_empty());
        assert_eq!(db.dev_mode("space_empty").await.unwrap().unwrap().branch, "main");

        // Every card kept, with its repo: from its facts, else its space's, else none
        let repo_of = |id: &'static str| {
            let db = &db;
            async move { db.dev_card(id).await.unwrap().unwrap().repo }
        };
        assert_eq!(repo_of("dvc_landed").await, "github.com/demo-org/app");
        assert_eq!(repo_of("dvc_review").await, "github.com/demo-org/app");
        assert_eq!(repo_of("dvc_notes").await, "github.com/demo-org/app", "facts that aren't JSON take the space's repo");
        assert_eq!(repo_of("dvc_orphan").await, "");
        let review = db.dev_card("dvc_review").await.unwrap().unwrap();
        assert_eq!((review.status.as_str(), review.posted.as_deref()), ("ready", Some("{\"url\":\"u\"}")), "status and posted survive");

        // Nothing is announced again: the same card can't be inserted twice
        assert!(db.insert_dev_card_in(SPACE, "github.com/demo-org/app", LANDED, "aaaa", "aaaa", "{}").await.unwrap().is_none());
        assert!(db.insert_dev_card_in(SPACE, "github.com/demo-org/app", REVIEW, "pr:12", "bbbb", "{}").await.unwrap().is_none());

        // #357's column came first: the badge keeps 0.7.1's seen time
        let nine: DateTime<Utc> = "2026-11-06T09:00:00Z".parse().unwrap();
        let mode = db.dev_mode(SPACE).await.unwrap().unwrap();
        assert_eq!((mode.seen_at, mode.reviews_seen_at), (Some(nine), Some(nine)), "backfilled from seen_at");
        // A Mark seen after the migration is never copied into the badge by a later start
        let later: DateTime<Utc> = "2026-11-06T11:00:00Z".parse().unwrap();
        db.dev_mode_seen(SPACE, later).await.unwrap();

        // Idempotent: a second start changes nothing
        db.migrate_dev_mode().await.unwrap();
        let mode = db.dev_mode(SPACE).await.unwrap().unwrap();
        assert_eq!((mode.seen_at, mode.reviews_seen_at), (Some(later), Some(nine)));
        assert_eq!(db.dev_repos(SPACE).await.unwrap(), rows);
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dev_cards").fetch_one(&db.pool).await.unwrap();
        assert_eq!(n, 4);
        let tables: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = 'dev_cards_v2'").fetch_one(&db.pool).await.unwrap();
        assert_eq!(tables, 0);
    }

    /// 0.7.1's own statements, copied from dbdff6b (src/db/dev_mode.rs), as it runs them.
    const V071_CARD_COLS: &str = "id, space_id, kind, ref, head_sha, status, facts, card, created_at, updated_at, posted, post_error";
    const V071_INSERT: &str = "INSERT INTO dev_cards (id, space_id, kind, ref, head_sha, status, facts, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'pending', ?, ?, ?) ON CONFLICT(space_id, kind, ref, head_sha) DO NOTHING";
    const V071_STALE: &str = "UPDATE dev_cards SET status = 'stale', updated_at = ?4
             WHERE space_id = ?1 AND kind = 'review' AND ref = ?2 AND status != 'stale' AND (?3 IS NULL OR head_sha != ?3) RETURNING id";
    const V071_FACTS: &str = "UPDATE dev_cards SET facts = ?2, status = 'pending', updated_at = ?3 WHERE id = ?1";
    const V071_SET: &str = "UPDATE dev_cards SET status = ?2, card = COALESCE(?3, card), updated_at = ?4 WHERE id = ?1 AND status != 'stale'";
    const V071_POLLED: &str = "UPDATE dev_mode_spaces SET last_seen_sha = COALESCE(?2, last_seen_sha), polled_at = ?3, poll_error = ?4 WHERE space_id = ?1";
    /// 0.7.1's one seen route (desk opened and Mark seen alike).
    const V071_SEEN: &str = "UPDATE dev_mode_spaces SET seen_at = ?2 WHERE space_id = ?1";

    async fn v071_stale(db: &Database, ref_: &str, keep: Option<&str>) -> Vec<String> {
        let rows: Vec<(String,)> =
            sqlx::query_as(V071_STALE).bind(SPACE).bind(ref_).bind(keep).bind(Utc::now()).fetch_all(&db.pool).await.unwrap();
        rows.into_iter().map(|r| r.0).collect()
    }

    async fn v071_facts(db: &Database, id: &str, facts: &str) {
        sqlx::query(V071_FACTS).bind(id).bind(facts).bind(Utc::now()).execute(&db.pool).await.unwrap();
    }

    #[tokio::test]
    async fn a_071_build_on_a_migrated_database_only_stops_making_cards_and_coming_back_catches_up() {
        let (db, _tmp) = db().await;
        as_071(&db).await;
        db.migrate_dev_mode().await.unwrap();
        // 0.7.2 then watched a second repo, with its own PR #12 and a landed card
        let site = "github.com/demo-org/site";
        let site_review = db.insert_dev_card_in(SPACE, site, REVIEW, "pr:12", "ssss", "{\"repo\":\"demo-org/site\"}").await.unwrap().unwrap();
        db.set_dev_card(&site_review.id, READY, Some("{}")).await.unwrap().unwrap();
        let site_landed = db.insert_dev_card_in(SPACE, site, LANDED, "s111", "s111", "{\"ci\":\"pending\"}").await.unwrap().unwrap();
        db.set_dev_card(&site_landed.id, READY, Some("{}")).await.unwrap().unwrap();
        let before: Vec<(String, String, String, String)> =
            sqlx::query_as("SELECT id, repo, status, facts FROM dev_cards ORDER BY id").fetch_all(&db.pool).await.unwrap();

        // --- Downgrade: 0.7.1 starts. Its migration is a no-op on this shape
        for sql in [
            "CREATE TABLE IF NOT EXISTS dev_cards (id TEXT PRIMARY KEY, space_id TEXT NOT NULL, kind TEXT NOT NULL, ref TEXT NOT NULL,
               head_sha TEXT NOT NULL, status TEXT NOT NULL, facts TEXT NOT NULL, card TEXT,
               created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
               UNIQUE(space_id, kind, ref, head_sha))",
            "CREATE INDEX IF NOT EXISTS idx_dev_cards_space ON dev_cards(space_id, kind, created_at)",
        ] {
            sqlx::query(sql).execute(&db.pool).await.unwrap();
        }
        // It reads every card, every repo's
        let read = sqlx::query(&format!("SELECT {V071_CARD_COLS} FROM dev_cards WHERE space_id = ?")).bind(SPACE).fetch_all(&db.pool).await.unwrap();
        assert_eq!(read.len(), 5);
        // Its insert fails (the poller logs it and goes on): no card is made, none is lost
        let now = Utc::now();
        let insert = sqlx::query(V071_INSERT)
            .bind("dvc_071").bind(SPACE).bind(LANDED).bind("dddd").bind("dddd").bind("{}").bind(now).bind(now)
            .execute(&db.pool)
            .await;
        assert!(insert.is_err(), "0.7.1 can't make cards here");
        // Its PR #12 moved on: only its own repo's old card goes stale, not demo-org/site's
        assert_eq!(v071_stale(&db, "pr:12", Some("bnew")).await, vec!["dvc_review".to_string()]);
        // Its "PR closed" sweep over every review card can't touch the other repo's either
        assert!(v071_stale(&db, "pr:12", None).await.is_empty());
        // Its CI recheck can't rewrite another repo's facts; its own repo's still work
        v071_facts(&db, &site_landed.id, "{\"ci\":\"wrong repo\"}").await;
        v071_facts(&db, "dvc_landed", "{\"repo\":\"Demo-Org/App\",\"ci\":\"pass\"}").await;
        assert_eq!(db.dev_card("dvc_landed").await.unwrap().unwrap().status, PENDING);
        // Finishing a card by id still works for any repo (the person's edits, generate)
        sqlx::query(V071_SET).bind("dvc_landed").bind(READY).bind(None::<String>).bind(Utc::now()).execute(&db.pool).await.unwrap();
        // It polls on, moving only its own 0.7.1 place
        sqlx::query(V071_POLLED).bind(SPACE).bind("zzzz").bind(Utc::now()).bind(None::<String>).execute(&db.pool).await.unwrap();
        // It reads the space row with the extra column, and its seen route moves only seen_at
        let badge = db.dev_mode(SPACE).await.unwrap().unwrap().reviews_seen_at;
        assert!(badge.is_some());
        let opened_on_071 = Utc::now();
        sqlx::query(V071_SEEN).bind(SPACE).bind(opened_on_071).execute(&db.pool).await.unwrap();

        let after: Vec<(String, String, String, String)> =
            sqlx::query_as("SELECT id, repo, status, facts FROM dev_cards ORDER BY id").fetch_all(&db.pool).await.unwrap();
        let of = |rows: &[(String, String, String, String)], id: &str| rows.iter().find(|r| r.0 == id).cloned().unwrap();
        assert_eq!(after.len(), before.len());
        assert_eq!(of(&after, &site_review.id), of(&before, &site_review.id), "the other repo's review is untouched");
        assert_eq!(of(&after, &site_landed.id), of(&before, &site_landed.id), "the other repo's facts are untouched");
        assert_eq!(of(&after, "dvc_review").2, STALE);

        // --- Back up to 0.7.2: nothing is rebuilt, every row is as 0.7.1 left it
        db.migrate_dev_mode().await.unwrap();
        let again: Vec<(String, String, String, String)> =
            sqlx::query_as("SELECT id, repo, status, facts FROM dev_cards ORDER BY id").fetch_all(&db.pool).await.unwrap();
        assert_eq!(again, after);
        // #357's column isn't added or backfilled again: the badge keeps its time, the rollup 0.7.1's
        let mode = db.dev_mode(SPACE).await.unwrap().unwrap();
        assert_eq!((mode.seen_at, mode.reviews_seen_at), (Some(opened_on_071), badge));
        let cols: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info('dev_mode_spaces') WHERE name = 'reviews_seen_at'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(cols, 1);
        // 0.7.2 picks the repo up where it left it, so what landed meanwhile becomes cards, once
        let app = "github.com/demo-org/app";
        assert_eq!(db.dev_repo(SPACE, app).await.unwrap().unwrap().last_seen_sha.as_deref(), Some("aaaa"));
        let caught = db.insert_dev_card_in(SPACE, app, LANDED, "dddd", "dddd", "{}").await.unwrap();
        assert!(caught.is_some());
        assert!(db.insert_dev_card_in(SPACE, app, LANDED, "dddd", "dddd", "{}").await.unwrap().is_none());
        assert!(db.insert_dev_card_in(SPACE, app, REVIEW, "pr:12", "bnew", "{}").await.unwrap().is_some(), "PR #12's new head gets its card");
        // and 0.7.2's own writes to the other repo's cards go through the guard
        db.set_dev_card_facts(&site_landed.id, "{\"ci\":\"pass\"}").await.unwrap();
        assert_eq!(db.dev_card(&site_landed.id).await.unwrap().unwrap().facts, "{\"ci\":\"pass\"}");
        assert_eq!(db.stale_dev_reviews_in(SPACE, site, "pr:12", None).await.unwrap(), vec![site_review.id.clone()]);
    }

    /// Every Dev mode table's schema and rows, to compare two states of the database exactly.
    async fn dev_state(db: &Database) -> Vec<String> {
        let mut out: Vec<String> = sqlx::query_scalar(
            "SELECT type || ' ' || name || ' ' || COALESCE(sql, '') FROM sqlite_master WHERE name LIKE '%dev_%' ORDER BY type, name",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        for t in ["dev_mode_spaces", "dev_cards", "dev_repos"] {
            let has: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?").bind(t).fetch_one(&db.pool).await.unwrap();
            if has == 1 {
                let cols: Vec<String> = sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{t}') ORDER BY cid")).fetch_all(&db.pool).await.unwrap();
                let quoted: Vec<String> = cols.iter().map(|c| format!("quote(\"{c}\")")).collect();
                let dump: Vec<String> =
                    sqlx::query_scalar(&format!("SELECT {} FROM {t} ORDER BY 1", quoted.join(" || ',' || "))).fetch_all(&db.pool).await.unwrap();
                out.extend(dump.into_iter().map(|r| format!("{t}: {r}")));
            }
        }
        out
    }

    /// #357's `dev_mode_spaces.reviews_seen_at`, there or not, so both start orders are covered.
    async fn with_reviews_seen(db: &Database, there: bool) {
        let has: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info('dev_mode_spaces') WHERE name = 'reviews_seen_at'").fetch_one(&db.pool).await.unwrap();
        let sql = match (there, has == 1) {
            (true, false) => "ALTER TABLE dev_mode_spaces ADD COLUMN reviews_seen_at TIMESTAMP",
            (false, true) => "ALTER TABLE dev_mode_spaces DROP COLUMN reviews_seen_at",
            _ => return,
        };
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }

    #[tokio::test]
    async fn the_migration_runs_again_without_changing_anything_whether_357_ran_first_or_not() {
        for seen_first in [false, true] {
            let (db, _tmp) = db().await;
            as_071(&db).await;
            with_reviews_seen(&db, seen_first).await;
            db.migrate_dev_mode().await.unwrap();
            let once = dev_state(&db).await;
            assert!(once.iter().any(|l| l.starts_with("dev_repos: ")), "migrated (#357 first: {seen_first})");
            // A second and a third start change nothing and don't fail
            db.migrate_dev_mode().await.unwrap();
            db.migrate_dev_mode().await.unwrap();
            assert_eq!(dev_state(&db).await, once, "#357 first: {seen_first}");
        }
    }

    async fn same_clock(db: &Database) {
        sqlx::query("UPDATE dev_cards SET created_at = '2026-11-06 00:00:00', updated_at = '2026-11-06 00:00:00'")
            .execute(&db.pool).await.unwrap();
    }

    #[tokio::test]
    async fn a_migration_that_dies_halfway_leaves_the_071_shape_and_the_next_start_redoes_it() {
        let (db, _tmp) = db().await;
        as_071(&db).await;
        with_reviews_seen(&db, true).await;
        let before = dev_state(&db).await;
        // Halfway: the repo copy has run when making the new cards table fails (a name in the way
        // stands in for the crash). Everything the migration did must be undone.
        sqlx::query("CREATE TABLE dev_cards_v2 (x)").execute(&db.pool).await.unwrap();
        assert!(db.migrate_dev_mode().await.is_err());
        sqlx::query("DROP TABLE dev_cards_v2").execute(&db.pool).await.unwrap();
        let after: Vec<String> = dev_state(&db).await;
        // Only what runs before the transaction is new: the empty 0.7.2 tables and indexes that
        // 0.7.1 never reads (CREATE ... IF NOT EXISTS). No row was copied, no card was moved.
        let old: Vec<&String> = before.iter().collect();
        let kept: Vec<&String> = after.iter().filter(|l| !l.starts_with("table dev_repos") && !l.starts_with("index sqlite_autoindex_dev_repos")).collect();
        assert_eq!(kept, old, "the 0.7.1 shape is intact");
        assert!(!after.iter().any(|l| l.starts_with("dev_repos: ")), "the repo copy was rolled back");
        assert!(!db.dev_cards_has("repo").await.unwrap());
        // The next start redoes it cleanly, as if nothing had happened
        db.migrate_dev_mode().await.unwrap();
        let fresh = {
            let (clean, _t) = super::tests::db().await;
            as_071(&clean).await;
            with_reviews_seen(&clean, true).await;
            clean.migrate_dev_mode().await.unwrap();
            same_clock(&clean).await;
            dev_state(&clean).await
        };
        // The two copies were seeded a moment apart; only their cards' clocks may differ
        same_clock(&db).await;
        assert_eq!(dev_state(&db).await, fresh);
        assert_eq!(db.dev_repos(SPACE).await.unwrap().len(), 1);
        assert_eq!(db.dev_card("dvc_review").await.unwrap().unwrap().repo, "github.com/demo-org/app");
    }

    #[tokio::test]
    async fn a_fresh_database_needs_no_migration_and_reopens() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        db.put_dev_mode(SPACE, true, "demo-org/app", "main", None, None).await.unwrap();
        db.insert_dev_card_in(SPACE, "github.com/demo-org/app", LANDED, "aaaa", "aaaa", "{}").await.unwrap().unwrap();
        drop(db);
        let db = Database::init(tmp.path()).await.unwrap();
        assert_eq!(db.dev_repos(SPACE).await.unwrap().len(), 1);
        assert_eq!(db.dev_cards_in(SPACE, None, LANDED, None, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn pr_12_in_two_repos_is_two_cards() {
        let (db, _tmp) = db().await;
        let app = db.insert_dev_card_in(SPACE, "github.com/demo-org/app", REVIEW, "pr:12", "same", "{}").await.unwrap().unwrap();
        let site = db.insert_dev_card_in(SPACE, "github.com/demo-org/site", REVIEW, "pr:12", "same", "{}").await.unwrap().unwrap();
        assert_ne!(app.id, site.id);
        assert!(db.insert_dev_card_in(SPACE, "github.com/demo-org/app", REVIEW, "pr:12", "same", "{}").await.unwrap().is_none(), "the same repo still dedupes");
        // Each repo's lookups and stale marks see only its own card
        let of = db.dev_cards_of_ref_in(SPACE, "github.com/demo-org/app", REVIEW, "pr:12").await.unwrap();
        assert_eq!(of.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec![app.id.as_str()]);
        assert_eq!(db.dev_cards_in(SPACE, Some("github.com/demo-org/site"), REVIEW, None, 10).await.unwrap().len(), 1);
        assert_eq!(db.dev_cards_in(SPACE, None, REVIEW, None, 10).await.unwrap().len(), 2);
        let staled = db.stale_dev_reviews_in(SPACE, "github.com/demo-org/app", "pr:12", None).await.unwrap();
        assert_eq!(staled, vec![app.id.clone()]);
        assert_eq!(db.dev_card(&site.id).await.unwrap().unwrap().status, PENDING, "the other repo's PR #12 is untouched");
    }

    #[tokio::test]
    async fn a_space_watches_at_most_ten_repos() {
        let (db, _tmp) = db().await;
        for i in 0..MAX_REPOS {
            let key = repo_key("github.com", &format!("demo-org/r{i}"));
            assert_eq!(db.add_dev_repo(SPACE, &key, Some("t"), &format!("demo-org/r{i}"), Some("github.com"), "main", None, BY_TEAM).await.unwrap(), RepoAdd::Added);
        }
        let key = repo_key("github.com", "demo-org/eleven");
        assert_eq!(db.add_dev_repo(SPACE, &key, None, "demo-org/eleven", Some("github.com"), "main", None, BY_HAND).await.unwrap(), RepoAdd::TooMany);
        assert_eq!(db.dev_repos(SPACE).await.unwrap().len(), MAX_REPOS);
        // At the cap an existing key is still kept (a team label update), and another space has its own 10
        let first = repo_key("github.com", "demo-org/r0");
        assert_eq!(db.add_dev_repo(SPACE, &first, Some("t2"), "demo-org/r0", Some("github.com"), "main", None, BY_TEAM).await.unwrap(), RepoAdd::Kept);
        assert_eq!(db.dev_repo(SPACE, &first).await.unwrap().unwrap().team.as_deref(), Some("t2"));
        assert_eq!(db.add_dev_repo("space_other", &key, None, "demo-org/eleven", Some("github.com"), "main", None, BY_HAND).await.unwrap(), RepoAdd::Added);
        // Removing one makes room
        assert!(db.delete_dev_repo(SPACE, &first).await.unwrap());
        assert_eq!(db.add_dev_repo(SPACE, &key, None, "demo-org/eleven", Some("github.com"), "main", None, BY_HAND).await.unwrap(), RepoAdd::Added);
    }

    #[tokio::test]
    async fn a_new_branch_starts_the_watch_over_and_a_rekey_moves_the_cards() {
        let (db, _tmp) = db().await;
        let gh = repo_key("github.com", "demo-org/app");
        db.add_dev_repo(SPACE, &gh, None, "demo-org/app", Some("github.com"), "main", None, BY_MIGRATION).await.unwrap();
        db.dev_repo_polled(SPACE, &gh, Some("aaaa"), None).await.unwrap();
        assert!(db.set_dev_repo_branch(SPACE, &gh, "main").await.unwrap());
        assert_eq!(db.dev_repo(SPACE, &gh).await.unwrap().unwrap().last_seen_sha.as_deref(), Some("aaaa"), "the same branch keeps its place");
        db.insert_dev_card_in(SPACE, &gh, LANDED, "aaaa", "aaaa", "{}").await.unwrap().unwrap();
        let ghe = repo_key("ghe.example.com", "demo-org/app");
        assert!(db.rekey_dev_repo(SPACE, &gh, &ghe, "ghe.example.com").await.unwrap());
        assert_eq!(db.dev_cards_in(SPACE, Some(&ghe), LANDED, None, 10).await.unwrap().len(), 1);
        assert!(db.dev_repo(SPACE, &gh).await.unwrap().is_none());
        // A taken key is refused
        db.add_dev_repo(SPACE, &gh, None, "demo-org/app", Some("github.com"), "main", None, BY_HAND).await.unwrap();
        assert!(!db.rekey_dev_repo(SPACE, &gh, &ghe, "ghe.example.com").await.unwrap());
        assert!(db.set_dev_repo_branch(SPACE, &ghe, "release").await.unwrap());
        let r = db.dev_repo(SPACE, &ghe).await.unwrap().unwrap();
        assert_eq!((r.branch.as_str(), r.last_seen_sha.as_deref()), ("release", None), "a new branch starts over");
    }
}
