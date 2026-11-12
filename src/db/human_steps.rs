//! Steps only the person can do in the bot browser (`request_human_step`): sign in, solve a
//! CAPTCHA, or something else. A step is `open` until the person marks it done or declines it,
//! or its turn is stopped (`cancelled`). A CAPTCHA step also closes by itself when the check
//! is gone from the page (`resolved_auto`) or nobody solved it in time (`timed_out`).

use super::models::{Message, MessageExtra, NewMessage};
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

pub const MAX_REASON_CHARS: usize = 300;
pub const MAX_URL_CHARS: usize = 300;
/// Most open steps per agent; an agent can't flood the person.
pub const MAX_OPEN_PER_KWANT: i64 = 5;

pub const HUMAN_STEP_KINDS: [&str; 4] = ["sign_in", "captcha", "two_factor", "other"];
/// "open", "done", "declined", "cancelled" (its turn was stopped), "resolved_auto" (the
/// CAPTCHA went away on its own or the person solved it without pressing Done) or "timed_out".
pub const HUMAN_STEP_STATUSES: [&str; 6] = ["open", "done", "declined", "cancelled", "resolved_auto", "timed_out"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct HumanStep {
    pub id: String,
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    /// The turn that asked; it may have ended since (a late answer starts a new turn).
    pub turn_id: Option<String>,
    pub kind: String,
    pub reason: String,
    /// Where to do it: scheme, host and path only, never a query or a password.
    pub url: Option<String>,
    /// The bot browser it happens in (team, channel or kwant id).
    pub sandbox_target: Option<String>,
    /// CAPTCHA steps: where the widget is, `{kind, x, y, width, height}` in viewport pixels,
    /// kept as JSON text and sent as an object.
    #[serde(serialize_with = "target_object")]
    pub target: Option<String>,
    /// "bot" (the agent asked) or "auto" (kwnt saw a CAPTCHA).
    pub source: String,
    pub status: String,
    /// The chat message that shows the step.
    pub message_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    /// The page of the bot browser the step holds: a job's own page
    /// (`team#bot#chat`, see `server::browser_pages`) or the team page. `NULL` on steps saved
    /// before pages existed, which held the team page ([`Self::page`]).
    #[sqlx(default)]
    pub page_key: Option<String>,
}

fn target_object<S: serde::Serializer>(target: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    use serde::Serialize as _;
    target.as_deref().and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()).serialize(s)
}

impl HumanStep {
    /// The page this step holds: its own page key, else (a step saved before job pages) the
    /// team page, which is the sandbox target itself.
    pub fn page(&self) -> Option<&str> {
        self.page_key.as_deref().or(self.sandbox_target.as_deref())
    }

    /// The chat id the step was asked in: channel, thread or the direct chat's kwant.
    pub fn context_id(&self) -> String {
        self.channel_id
            .clone()
            .or_else(|| self.thread_id.clone())
            .or_else(|| self.direct_kwant_id.clone())
            .unwrap_or_else(|| self.kwant_id.clone())
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum HumanStepError {
    #[error("{0}")]
    Invalid(String),
    #[error("you already have {MAX_OPEN_PER_KWANT} steps waiting on the person; wait for them first")]
    TooManyOpen,
    #[error("database error")]
    Db,
}

impl From<DbError> for HumanStepError {
    fn from(_: DbError) -> Self {
        HumanStepError::Db
    }
}

impl From<sqlx::Error> for HumanStepError {
    fn from(_: sqlx::Error) -> Self {
        HumanStepError::Db
    }
}

/// The URL as it is kept and shown: http(s) only, without user info, query or fragment, which
/// can hold tokens. `None` when there is no usable URL.
pub fn clean_url(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    let s = url.to_string();
    (s.chars().count() <= MAX_URL_CHARS).then_some(s)
}

/// A request's kind, reason and URL, checked and trimmed.
pub fn clean_request(kind: &str, reason: &str, url: Option<&str>) -> Result<(String, String, Option<String>), HumanStepError> {
    let kind = kind.trim();
    if !HUMAN_STEP_KINDS.contains(&kind) {
        return Err(HumanStepError::Invalid("kind must be sign_in, captcha, two_factor or other".into()));
    }
    let reason = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    if reason.is_empty() {
        return Err(HumanStepError::Invalid("reason is required".into()));
    }
    if reason.chars().count() > MAX_REASON_CHARS {
        return Err(HumanStepError::Invalid(format!("reason is longer than {MAX_REASON_CHARS} characters")));
    }
    let url = match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => Some(clean_url(u).ok_or_else(|| HumanStepError::Invalid("url must be an http(s) address".into()))?),
        None => None,
    };
    Ok((kind.to_string(), reason, url))
}

/// The site a sign-in is for, as the prefix its kept URLs start with (`https://host[:port]/`).
/// `None` when the URL has no usable origin.
pub fn sign_in_site(url: &str) -> Option<String> {
    let origin = url::Url::parse(url).ok()?.origin();
    origin.is_tuple().then(|| format!("{}/", origin.ascii_serialization()))
}

/// Where a step is asked.
#[derive(Debug, Clone, Default)]
pub struct HumanStepPlace {
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub sandbox_target: Option<String>,
    /// The page it holds; `None` is the team page (`sandbox_target`).
    pub page_key: Option<String>,
}

/// A new step (with `true`), or one already open that the ask gets back (with `false`): the
/// turn's own, or the bot's open sign-in for the same site in the same chat.
pub type Created = (HumanStep, bool, Option<Message>);

/// SQL matching the bot's open sign-in for a site in one chat, from the placeholders of the bot,
/// channel, direct chat, thread, bot browser and site (`sign_in_site`; NULL matches nothing).
fn same_sign_in([kwant, channel, direct, thread, browser, site]: [&str; 6]) -> String {
    format!(
        "status = 'open' AND {site} IS NOT NULL AND kind = 'sign_in' AND kwant_id = {kwant} AND channel_id IS {channel} \
         AND direct_kwant_id IS {direct} AND thread_id IS {thread} AND sandbox_target IS {browser} \
         AND substr(url, 1, length({site})) = {site}"
    )
}

const CREATE_HUMAN_STEPS: &str = "CREATE TABLE IF NOT EXISTS human_steps (
    id TEXT PRIMARY KEY,
    kwant_id TEXT NOT NULL,
    channel_id TEXT,
    direct_kwant_id TEXT,
    thread_id TEXT,
    turn_id TEXT,
    kind TEXT NOT NULL CHECK (kind IN ('sign_in', 'captcha', 'two_factor', 'other')),
    reason TEXT NOT NULL,
    url TEXT,
    sandbox_target TEXT,
    target TEXT,
    source TEXT NOT NULL CHECK (source IN ('bot', 'auto')),
    status TEXT NOT NULL CHECK (status IN ('open', 'done', 'declined', 'cancelled', 'resolved_auto', 'timed_out')),
    message_id TEXT,
    created_at TIMESTAMP NOT NULL,
    closed_at TIMESTAMP,
    page_key TEXT
)";

impl Database {
    pub(super) async fn migrate_human_steps(&self) -> Result<(), DbError> {
        sqlx::query(CREATE_HUMAN_STEPS).execute(&self.pool).await?;
        // Tables made before CAPTCHA auto-resolve lack `target` and the two new statuses, which
        // a CHECK can't gain in place: rebuild them once
        let sql: Option<(String,)> = sqlx::query_as("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'human_steps'")
            .fetch_optional(&self.pool)
            .await?;
        if sql.is_some_and(|(sql,)| !sql.contains("'timed_out'")) {
            let columns = "id, kwant_id, channel_id, direct_kwant_id, thread_id, turn_id, kind, reason, url, sandbox_target, source, status, message_id, created_at, closed_at";
            let mut tx = self.pool.begin().await?;
            sqlx::query("DROP INDEX IF EXISTS human_steps_by_status").execute(&mut *tx).await?;
            sqlx::query("ALTER TABLE human_steps RENAME TO human_steps_old").execute(&mut *tx).await?;
            sqlx::query(CREATE_HUMAN_STEPS).execute(&mut *tx).await?;
            sqlx::query(&format!("INSERT INTO human_steps ({columns}) SELECT {columns} FROM human_steps_old"))
                .execute(&mut *tx)
                .await?;
            sqlx::query("DROP TABLE human_steps_old").execute(&mut *tx).await?;
            tx.commit().await?;
        }
        // 0.9.3: the page a step holds (additive; older rows keep NULL, the team page)
        let has_page: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info('human_steps') WHERE name = 'page_key'")
            .fetch_one(&self.pool)
            .await?;
        if has_page == 0 {
            sqlx::query("ALTER TABLE human_steps ADD COLUMN page_key TEXT").execute(&self.pool).await?;
        }
        sqlx::query("CREATE INDEX IF NOT EXISTS human_steps_by_status ON human_steps (status, kwant_id)").execute(&self.pool).await?;
        Ok(())
    }

    /// Saves a step together with the chat message that shows it (built by `card`), in one
    /// transaction. A turn that asks again while its step is open gets that step back, so a
    /// retrying agent (or the CAPTCHA check firing on every page) doesn't pile up cards. A sign-in
    /// also comes back to another turn of the same bot in the same chat while it is open for the
    /// same site (scheme, host and port): two turns running at once (the person's, and one a
    /// closed CAPTCHA step started) would otherwise show the person two identical cards.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_human_step_with_card(
        &self,
        place: &HumanStepPlace,
        kind: &str,
        reason: &str,
        url: Option<&str>,
        target: Option<&str>,
        source: &str,
        card: impl FnOnce(&HumanStep) -> Option<(NewMessage, MessageExtra)>,
    ) -> Result<Created, HumanStepError> {
        let mut tx = self.pool.begin().await?;
        let get = "SELECT * FROM human_steps WHERE id = ?";
        let site = if kind == "sign_in" { url.and_then(sign_in_site) } else { None };
        // Writes first so the transaction holds SQLite's write lock before it reads
        let id = Uuid::new_v4().to_string();
        let inserted = sqlx::query(&format!(
            "INSERT INTO human_steps (id, kwant_id, channel_id, direct_kwant_id, thread_id, turn_id, kind, reason, url, sandbox_target, source, status, created_at, target, page_key)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'open', ?12, ?14, ?16
             WHERE (SELECT COUNT(*) FROM human_steps WHERE kwant_id = ?2 AND status = 'open') < ?13
               AND NOT EXISTS (SELECT 1 FROM human_steps WHERE status = 'open' AND ?6 IS NOT NULL AND turn_id = ?6)
               AND NOT EXISTS (SELECT 1 FROM human_steps WHERE {})",
            same_sign_in(["?2", "?3", "?4", "?5", "?10", "?15"])
        ))
        .bind(&id)
        .bind(&place.kwant_id)
        .bind(&place.channel_id)
        .bind(&place.direct_kwant_id)
        .bind(&place.thread_id)
        .bind(&place.turn_id)
        .bind(kind)
        .bind(reason)
        .bind(url)
        .bind(&place.sandbox_target)
        .bind(source)
        .bind(Utc::now())
        .bind(MAX_OPEN_PER_KWANT)
        .bind(target)
        .bind(&site)
        .bind(&place.page_key)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let mut existing = match &place.turn_id {
                Some(t) => {
                    sqlx::query_as::<_, HumanStep>("SELECT * FROM human_steps WHERE status = 'open' AND turn_id = ? LIMIT 1")
                        .bind(t)
                        .fetch_optional(&mut *tx)
                        .await?
                }
                None => None,
            };
            if existing.is_none() && site.is_some() {
                existing = sqlx::query_as::<_, HumanStep>(&format!(
                    "SELECT * FROM human_steps WHERE {} ORDER BY created_at LIMIT 1",
                    same_sign_in(["?1", "?2", "?3", "?4", "?5", "?6"])
                ))
                .bind(&place.kwant_id)
                .bind(&place.channel_id)
                .bind(&place.direct_kwant_id)
                .bind(&place.thread_id)
                .bind(&place.sandbox_target)
                .bind(&site)
                .fetch_optional(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            return match existing {
                Some(step) => Ok((step, false, None)),
                None => Err(HumanStepError::TooManyOpen),
            };
        }
        let mut step = sqlx::query_as::<_, HumanStep>(get).bind(&id).fetch_one(&mut *tx).await?;
        let mut message = None;
        if let Some((msg, extra)) = card(&step) {
            let saved = super::insert_message_on(&mut tx, &msg, &extra).await?;
            sqlx::query("UPDATE human_steps SET message_id = ? WHERE id = ?").bind(&saved.id).bind(&id).execute(&mut *tx).await?;
            step = sqlx::query_as::<_, HumanStep>(get).bind(&id).fetch_one(&mut *tx).await?;
            message = Some(saved);
        }
        tx.commit().await?;
        Ok((step, true, message))
    }

    pub async fn get_human_step(&self, id: &str) -> Result<Option<HumanStep>, DbError> {
        Ok(sqlx::query_as::<_, HumanStep>("SELECT * FROM human_steps WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Steps, newest first, at most 100: by status and/or chat (channel, thread or direct-chat
    /// kwant id); `space` keeps the steps of that space's kwants (all when `None`).
    pub async fn list_human_steps(
        &self,
        status: Option<&str>,
        context_id: Option<&str>,
        space: Option<&super::spaces::SpaceId>,
    ) -> Result<Vec<HumanStep>, DbError> {
        Ok(sqlx::query_as::<_, HumanStep>(
            "SELECT * FROM human_steps
             WHERE (?1 IS NULL OR status = ?1)
               AND (?2 IS NULL OR channel_id = ?2 OR thread_id = ?2 OR direct_kwant_id = ?2)
               AND (?3 IS NULL OR COALESCE((SELECT k.space_id FROM kwants k WHERE k.id = human_steps.kwant_id), 'spc_default') = ?3)
             ORDER BY created_at DESC LIMIT 100",
        )
        .bind(status)
        .bind(context_id)
        .bind(space.map(|s| s.as_str()))
        .fetch_all(&self.pool)
        .await?)
    }

    /// Whether the turn asked the person for any step (open or answered).
    pub async fn turn_asked_human_step(&self, turn_id: &str) -> Result<bool, DbError> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM human_steps WHERE turn_id = ?").bind(turn_id).fetch_one(&self.pool).await?;
        Ok(n > 0)
    }

    /// Whether the person was already asked to sign in (or for a code) on `site` in this browser
    /// page since `since`, whatever they answered: kwnt asks for a wall it finds once, not every turn.
    pub async fn sign_in_asked_since(&self, page: &str, site: &str, since: DateTime<Utc>) -> Result<bool, DbError> {
        let n: i64 = sqlx::query_scalar(
            // Numbered throughout: sqlx gives a bare `?` after `?2` the next unnumbered argument (the 2nd)
            "SELECT COUNT(*) FROM human_steps WHERE kind IN ('sign_in', 'two_factor') AND COALESCE(page_key, sandbox_target) = ?1 \
             AND substr(url, 1, length(?2)) = ?2 AND created_at >= ?3",
        )
        .bind(page)
        .bind(site)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// The open steps of one turn.
    pub async fn open_human_steps_for_turn(&self, turn_id: &str) -> Result<Vec<HumanStep>, DbError> {
        Ok(sqlx::query_as::<_, HumanStep>("SELECT * FROM human_steps WHERE status = 'open' AND turn_id = ?")
            .bind(turn_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Whether a step is open on this browser page (it stays the person's until then): a job's
    /// page key, or a sandbox target for its team page. A step saved before job pages (no
    /// `page_key`) holds its team page.
    pub async fn human_step_open_on(&self, page: &str) -> Result<bool, DbError> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM human_steps WHERE status = 'open' AND COALESCE(page_key, sandbox_target) = ?")
            .bind(page)
            .fetch_one(&self.pool)
            .await?;
        Ok(n > 0)
    }

    /// Moves an open step's CAPTCHA box (the widget showed up, or moved).
    pub async fn set_human_step_target(&self, id: &str, target: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE human_steps SET target = ? WHERE id = ? AND status = 'open' AND target IS NOT ?")
            .bind(target)
            .bind(id)
            .bind(target)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Closes an open step; `false` when it wasn't open any more (someone else closed it).
    pub async fn close_human_step(&self, id: &str, status: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE human_steps SET status = ?, closed_at = ? WHERE id = ? AND status = 'open'")
            .bind(status)
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_sign_in_wall_is_asked_once_per_page_and_site() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let place = HumanStepPlace {
            kwant_id: "k".into(),
            direct_kwant_id: Some("k".into()),
            turn_id: Some("t1".into()),
            sandbox_target: Some("k".into()),
            page_key: Some("k#job1".into()),
            ..Default::default()
        };
        let hour_ago = Utc::now() - chrono::Duration::hours(1);
        assert!(!db.turn_asked_human_step("t1").await.unwrap());
        assert!(!db.sign_in_asked_since("k#job1", "https://www.tiktok.com/", hour_ago).await.unwrap());
        let (s, _, _) = db
            .create_human_step_with_card(&place, "sign_in", "Ben needs you to sign in", Some("https://www.tiktok.com/login"), None, "auto", |_| None)
            .await
            .unwrap();
        assert!(db.turn_asked_human_step("t1").await.unwrap());
        assert!(!db.turn_asked_human_step("t2").await.unwrap());
        assert!(db.sign_in_asked_since("k#job1", "https://www.tiktok.com/", hour_ago).await.unwrap());
        // Whatever they answered: a declined ask isn't repeated within the window either
        assert!(db.close_human_step(&s.id, "declined").await.unwrap());
        assert!(db.sign_in_asked_since("k#job1", "https://www.tiktok.com/", hour_ago).await.unwrap());
        // Another page, another site, or outside the window: ask
        assert!(!db.sign_in_asked_since("k", "https://www.tiktok.com/", hour_ago).await.unwrap());
        assert!(!db.sign_in_asked_since("k#job1", "https://x.com/", hour_ago).await.unwrap());
        assert!(!db.sign_in_asked_since("k#job1", "https://www.tiktok.com/", Utc::now() + chrono::Duration::hours(1)).await.unwrap());
    }

    #[test]
    fn requests_are_checked_and_urls_lose_their_secrets() {
        assert!(clean_request("sign_in", "Log in to GitHub", None).is_ok());
        assert!(clean_request("login", "Log in", None).is_err());
        assert!(clean_request("other", "  ", None).is_err());
        assert!(clean_request("other", &"r".repeat(301), None).is_err());
        assert!(clean_request("captcha", "Solve it", Some("javascript:alert(1)")).is_err());
        let (_, reason, url) =
            clean_request("sign_in", " Log in\n to  GitHub ", Some("https://me:pw@github.com/login?return_to=x&token=abc#f")).unwrap();
        assert_eq!(reason, "Log in to GitHub");
        assert_eq!(url.as_deref(), Some("https://github.com/login"));
    }

    #[tokio::test]
    async fn a_step_closes_once_and_a_turn_gets_its_open_step_back() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let place = |turn: &str| HumanStepPlace {
            kwant_id: "k".into(),
            direct_kwant_id: Some("k".into()),
            turn_id: Some(turn.into()),
            sandbox_target: Some("k".into()),
            ..Default::default()
        };
        let (s, new, _) = db.create_human_step_with_card(&place("t1"), "sign_in", "Log in", None, None, "bot", |_| None).await.unwrap();
        assert!(new);
        assert_eq!(s.context_id(), "k");
        assert!(db.human_step_open_on("k").await.unwrap());
        let (again, new, _) = db.create_human_step_with_card(&place("t1"), "captcha", "Solve", None, None, "auto", |_| None).await.unwrap();
        assert!(!new);
        assert_eq!(again.id, s.id);
        assert_eq!(db.open_human_steps_for_turn("t1").await.unwrap().len(), 1);
        assert!(db.close_human_step(&s.id, "done").await.unwrap());
        assert!(!db.close_human_step(&s.id, "declined").await.unwrap());
        let closed = db.get_human_step(&s.id).await.unwrap().unwrap();
        assert_eq!(closed.status, "done");
        assert!(closed.closed_at.is_some());
        assert!(!db.human_step_open_on("k").await.unwrap());
        for i in 0..MAX_OPEN_PER_KWANT {
            db.create_human_step_with_card(&place(&format!("t{}", i + 10)), "other", "Do it", None, None, "bot", |_| None).await.unwrap();
        }
        let full = db.create_human_step_with_card(&place("t99"), "other", "Do it", None, None, "bot", |_| None).await;
        assert_eq!(full.err(), Some(HumanStepError::TooManyOpen));
        assert_eq!(db.list_human_steps(Some("open"), Some("k"), None).await.unwrap().len() as i64, MAX_OPEN_PER_KWANT);
    }

    #[test]
    fn a_sign_in_site_is_its_scheme_host_and_port() {
        assert_eq!(sign_in_site("https://www.instagram.com/accounts/login/").as_deref(), Some("https://www.instagram.com/"));
        assert_eq!(sign_in_site("https://example.com:8443/login").as_deref(), Some("https://example.com:8443/"));
        assert_eq!(sign_in_site("https://example.com:443/login").as_deref(), Some("https://example.com/"));
        assert_eq!(sign_in_site("not a url"), None);
    }

    #[tokio::test]
    async fn an_open_sign_in_comes_back_to_another_turn_of_the_bot_in_the_same_chat_for_the_same_site() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let in_chat = |kwant: &str, channel: &str, turn: &str| HumanStepPlace {
            kwant_id: kwant.into(),
            channel_id: Some(channel.into()),
            turn_id: Some(turn.into()),
            sandbox_target: Some(channel.into()),
            ..Default::default()
        };
        let ask = |place: HumanStepPlace, kind: &'static str, url: Option<&'static str>| {
            let db = db.clone();
            async move { db.create_human_step_with_card(&place, kind, "Sign in", url, None, "bot", |_| None).await.unwrap() }
        };
        let login = Some("https://www.instagram.com/accounts/login/");
        let (first, new, _) = ask(in_chat("k", "c", "t1"), "sign_in", login).await;
        assert!(new);
        // Another turn, same bot, chat and site (another page of it): the same step, no new card
        let (again, new, card) = ask(in_chat("k", "c", "t2"), "sign_in", Some("https://www.instagram.com/")).await;
        assert!(!new && card.is_none());
        assert_eq!(again.id, first.id);
        assert_eq!(again.turn_id.as_deref(), Some("t1"), "it stays the first turn's step");
        // Anything else is its own step: another site, scheme or port, another chat, another bot,
        // another kind, or a sign-in without a URL
        for (place, kind, url) in [
            (in_chat("k", "c", "t3"), "sign_in", Some("https://www.tiktok.com/login")),
            (in_chat("k", "c", "t4"), "sign_in", Some("http://www.instagram.com/accounts/login/")),
            (in_chat("k", "c", "t5"), "sign_in", Some("https://www.instagram.com:8443/accounts/login/")),
            (in_chat("k", "c2", "t6"), "sign_in", login),
            (in_chat("k2", "c", "t7"), "sign_in", login),
            (in_chat("k2", "c", "t8"), "two_factor", login),
            (in_chat("k3", "c", "t9"), "sign_in", None),
            (in_chat("k3", "c", "t10"), "sign_in", None),
        ] {
            let (s, new, _) = ask(place, kind, url).await;
            assert!(new, "{kind} {url:?} in {:?} for {}", s.channel_id, s.kwant_id);
        }
        // Once the person declined it, a later ask is a new step (the bot was told not to ask)
        assert!(db.close_human_step(&first.id, "declined").await.unwrap());
        let (after, new, _) = ask(in_chat("k", "c", "t11"), "sign_in", login).await;
        assert!(new);
        assert_ne!(after.id, first.id);
    }

    #[tokio::test]
    async fn old_tables_gain_target_and_the_auto_statuses_and_keep_their_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        // Recreate the table as it was before CAPTCHA auto-resolve, with one row
        sqlx::query("DROP TABLE human_steps").execute(&db.pool).await.unwrap();
        sqlx::query(
            "CREATE TABLE human_steps (id TEXT PRIMARY KEY, kwant_id TEXT NOT NULL, channel_id TEXT, direct_kwant_id TEXT,
                thread_id TEXT, turn_id TEXT, kind TEXT NOT NULL CHECK (kind IN ('sign_in', 'captcha', 'two_factor', 'other')),
                reason TEXT NOT NULL, url TEXT, sandbox_target TEXT, source TEXT NOT NULL CHECK (source IN ('bot', 'auto')),
                status TEXT NOT NULL CHECK (status IN ('open', 'done', 'declined', 'cancelled')), message_id TEXT,
                created_at TIMESTAMP NOT NULL, closed_at TIMESTAMP)",
        ).execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO human_steps (id, kwant_id, kind, reason, source, status, created_at) VALUES ('old', 'k', 'captcha', 'Solve', 'auto', 'open', ?)")
            .bind(Utc::now()).execute(&db.pool).await.unwrap();
        db.migrate_human_steps().await.unwrap();
        db.migrate_human_steps().await.unwrap();
        let old = db.get_human_step("old").await.unwrap().unwrap();
        assert_eq!((old.status.as_str(), old.target), ("open", None));
        assert!(db.set_human_step_target("old", r#"{"kind":"tiktok","x":1,"y":2,"width":30,"height":40}"#).await.unwrap());
        assert!(db.close_human_step("old", "resolved_auto").await.unwrap());
        let json = serde_json::to_value(db.get_human_step("old").await.unwrap().unwrap()).unwrap();
        assert_eq!(json["status"], "resolved_auto");
        assert_eq!(json["target"]["kind"], "tiktok");
        assert_eq!(json["target"]["width"], 30);
        let place = HumanStepPlace { kwant_id: "k".into(), turn_id: Some("t".into()), ..Default::default() };
        let (s, ..) = db.create_human_step_with_card(&place, "captcha", "Solve", None, None, "auto", |_| None).await.unwrap();
        assert!(db.close_human_step(&s.id, "timed_out").await.unwrap());
        assert!(serde_json::to_value(db.get_human_step(&s.id).await.unwrap().unwrap()).unwrap()["target"].is_null());
    }

    /// 0.9.3: a step holds only its own page; a step saved before pages (no page key, as an
    /// older kwnt left it) holds its team page, and a table from before the column gains it.
    #[tokio::test]
    async fn a_step_holds_its_page_and_old_steps_hold_the_team_page() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        // a table as 0.9.2 left it: no page_key column, one open step on team "t"
        sqlx::query("DROP TABLE human_steps").execute(&db.pool).await.unwrap();
        sqlx::query(&CREATE_HUMAN_STEPS.replace(",\n    page_key TEXT", "")).execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO human_steps (id, kwant_id, kind, reason, sandbox_target, source, status, created_at) VALUES ('old', 'k', 'sign_in', 'Log in', 't', 'bot', 'open', ?)")
            .bind(Utc::now()).execute(&db.pool).await.unwrap();
        // Migrate the way kwnt starts: reopen, so the one-connection migration pool runs it. Changing
        // the table under this test's runtime pool would leave another pooled connection with a
        // `SELECT *` column list from before (sqlx's index-out-of-bounds panic, then None).
        db.pool.close().await;
        let db = Database::init(tmp.path()).await.unwrap();
        let old = db.get_human_step("old").await.unwrap().unwrap();
        assert_eq!(old.page_key, None);
        assert_eq!(old.page(), Some("t"), "an old step holds its team page");
        assert!(db.human_step_open_on("t").await.unwrap());
        assert!(!db.human_step_open_on("t#k#chat").await.unwrap(), "a job page is free while the team page is held");
        assert!(db.close_human_step("old", "done").await.unwrap());
        // a job's step holds that job's page only
        let place = HumanStepPlace {
            kwant_id: "k".into(),
            thread_id: Some("chat".into()),
            turn_id: Some("turn".into()),
            sandbox_target: Some("t".into()),
            page_key: Some("t#k#chat".into()),
            ..Default::default()
        };
        let (s, _, _) = db.create_human_step_with_card(&place, "sign_in", "Log in", None, None, "bot", |_| None).await.unwrap();
        assert_eq!(s.page(), Some("t#k#chat"));
        assert!(db.human_step_open_on("t#k#chat").await.unwrap());
        assert!(!db.human_step_open_on("t").await.unwrap(), "the team page stays free");
        assert!(!db.human_step_open_on("t#k#other").await.unwrap());
        // migrating again changes nothing
        db.migrate_human_steps().await.unwrap();
        assert_eq!(db.get_human_step(&s.id).await.unwrap().unwrap().page_key.as_deref(), Some("t#k#chat"));
    }
}
