//! sign-in-wall-uses-tools B and C: a bot whose browser meets a site it already has MCP tools for
//! (a `github` server and github.com) is pointed at those tools, instead of the person being asked
//! to sign in to the site in the bot browser.
//!
//! B: a turn ended on that site's sign-in page. When the tools work, the bot gets one follow-up
//! turn that tells it to use them (at most once per bot, chat and site in [`FOLLOW_UP_WINDOW`], so a
//! follow-up that ends on the same wall never starts another). When they're failing, the person
//! gets one step that names the real problem (the saved key was rejected) and where to fix it,
//! once per bot and site every [`super::human_steps::SIGN_IN_REASK_HOURS`].
//!
//! C: a browser page lands on such a site: its result says once per turn and site that the tools
//! are there (or that they're failing and the person should fix them in Tools).

use super::continuity::SelfScope;
use super::routes::scope_for;
use super::AppState;
use crate::db::models::McpServer;
use crate::mcp::client::UNAUTHORIZED_KEY_HINT;
use crate::mcp::covers;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A covered site's follow-up turn is started at most once per bot, chat and site in this window
/// (or until the person writes in that chat again).
pub const FOLLOW_UP_WINDOW: Duration = Duration::from_secs(60 * 60);
/// How long a browser hint is remembered for its turn (a turn never runs this long).
const HINT_WINDOW: Duration = Duration::from_secs(6 * 60 * 60);
/// A last Tools-screen test younger than this answers whether a server works; an older one (or
/// none) is tested again, bounded by [`TEST_LIMIT`].
const CHECK_FRESH: Duration = Duration::from_secs(10 * 60);
const TEST_LIMIT: Duration = Duration::from_secs(6);

/// Whether a covering server works, as its last test said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Works,
    /// A 401/403 on a key the person saved.
    KeyRejected,
    /// kwnt's sign-in to the server is gone (OAuth): sign in again from Tools.
    SignedOut,
    /// Any other failure (not reachable, didn't start, …).
    NotConnecting,
}

/// A test's outcome (`ok`, `auth_required`, `error`, as the Tools screen gets it) as [`Health`].
pub fn health_of(ok: bool, auth_required: bool, error: Option<&str>) -> Health {
    if ok {
        Health::Works
    } else if auth_required {
        Health::SignedOut
    } else if error.is_some_and(|e| e.contains(UNAUTHORIZED_KEY_HINT) || e.contains("401") || e.contains("403")) {
        Health::KeyRejected
    } else {
        Health::NotConnecting
    }
}

/// A server's health from its last test only (no network): `None` when it hasn't been tested
/// since kwnt started, or (with `fresh`) its last test is older than that.
fn cached_health(server_id: &str, fresh: Option<Duration>) -> Option<Health> {
    let (ok, auth_required, error, at) = super::connections::last_mcp_check(server_id)?;
    if let Some(fresh) = fresh {
        let age = (chrono::Utc::now() - at).to_std().unwrap_or_default();
        if age > fresh {
            return None;
        }
    }
    Some(health_of(ok, auth_required, error.as_deref()))
}

/// A server's health: its recent last test, else the Tools screen's test run now (bounded; a test
/// that runs out of time counts as working, and the follow-up turn hears any real problem).
async fn health(state: &AppState, server: &McpServer) -> Health {
    if let Some(h) = cached_health(&server.id, Some(CHECK_FRESH)) {
        return h;
    }
    match tokio::time::timeout(TEST_LIMIT, super::mcp_routes::run_test(state, server)).await {
        Ok(outcome) => health_of(outcome["ok"] == Value::Bool(true), outcome["auth_required"] == Value::Bool(true), outcome["error"].as_str()),
        Err(_) => Health::Works,
    }
}

/// The servers to name and how they are: every working one, or (none works) the first failing one.
pub fn pick(checked: &[(String, Health)]) -> (Vec<String>, Health) {
    let working: Vec<String> = checked.iter().filter(|(_, h)| *h == Health::Works).map(|(n, _)| n.clone()).collect();
    if !working.is_empty() {
        return (working, Health::Works);
    }
    match checked.first() {
        Some((name, h)) => (vec![name.clone()], *h),
        None => (Vec::new(), Health::Works),
    }
}

fn ticked(names: &[String]) -> String {
    names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
}

/// The follow-up turn's prompt (B, the tools work).
pub fn follow_up_prompt(host: &str, names: &[String]) -> String {
    let tools = ticked(names);
    format!(
        "[Your browser stopped at {host}'s sign-in page]\nYou have the {tools} tools for {host}, so you don't need to sign in \
         there. Carry on with the task using those tools instead of the browser. Only if they can't do what's needed, ask the \
         person with request_human_step, saying exactly what you need from them."
    )
}

/// The person's step when the covering tools are failing (B): what is wrong, where to fix it,
/// and that Done starts the bot again.
pub fn failing_reason(kwant_name: &str, server: &str, host: &str, health: Health) -> String {
    // "has <server> tools for <host>" first: the card shows the host right under it, so the
    // reason says why there's no sign-in there (Mira's copy check D1-D3)
    let reason = match health {
        Health::SignedOut => format!(
            "{kwant_name} has {server} tools for {host}, but their sign-in has run out. Sign in to {server} again in Tools, then press Done."
        ),
        Health::KeyRejected => format!(
            "{kwant_name} has {server} tools for {host}, but the secret saved for {server} was rejected. Edit {server} in Tools and paste a new secret, then press Done."
        ),
        Health::NotConnecting | Health::Works => format!(
            "{kwant_name} has {server} tools for {host}, but they aren't connecting. Press Test connection on {server} in Tools to see why, then press Done."
        ),
    };
    reason.chars().take(300).collect()
}

/// The line a browser result gets on a covered site (C).
pub fn browser_hint_text(names: &[String], host: &str, health: Health) -> String {
    let tools = ticked(names);
    let why = match health {
        Health::Works => return format!("You have the {tools} tools for {host}; use them rather than the browser."),
        Health::KeyRejected => "the secret saved for it was rejected",
        Health::SignedOut => "their sign-in has run out",
        Health::NotConnecting => "they aren't connecting",
    };
    format!(
        "You have the {tools} tools for {host}, but they can't connect right now: {why}. Tell the person to fix it in Tools rather \
         than asking them to sign in to {host} in the browser."
    )
}

fn memo(cell: &'static OnceLock<Mutex<HashMap<String, Instant>>>) -> &'static Mutex<HashMap<String, Instant>> {
    cell.get_or_init(Default::default)
}

/// `true` the first time `key` is seen in `window` (and remembers it); old keys are dropped.
fn first_time(map: &Mutex<HashMap<String, Instant>>, key: String, window: Duration) -> bool {
    let mut seen = map.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    seen.retain(|_, at| now.duration_since(*at) < window);
    if seen.contains_key(&key) {
        return false;
    }
    seen.insert(key, now);
    true
}

static FOLLOWED_UP: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
static HINTED: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

fn follow_up_key(kwant: &str, chat: &str, site: &str) -> String {
    format!("{kwant}\u{1f}{chat}\u{1f}{site}")
}

/// Whether this bot may get a follow-up turn for `site` in `chat` now (and counts it).
pub fn may_follow_up(kwant: &str, chat: &str, site: &str) -> bool {
    first_time(memo(&FOLLOWED_UP), follow_up_key(kwant, chat, site), FOLLOW_UP_WINDOW)
}

/// The person wrote in `chat` (D): a new chain of turns starts, so a sign-in wall in it may get
/// its own follow-up.
pub fn forget_chat(kwant: &str, chat: &str) {
    let prefix = format!("{kwant}\u{1f}{chat}\u{1f}");
    memo(&FOLLOWED_UP).lock().unwrap_or_else(|e| e.into_inner()).retain(|k, _| !k.starts_with(&prefix));
}

/// Whether this turn hasn't been told about `label`'s tools yet (and counts it).
fn may_hint(turn: &str, label: &str) -> bool {
    first_time(memo(&HINTED), format!("{turn}\u{1f}{label}"), HINT_WINDOW)
}

/// B: a turn ended on `host`'s sign-in page (`url`, site `site`) and `servers` cover it. Starts
/// the follow-up turn or raises the tools step; returns the prompt or the step's reason, `None`
/// when it did neither (already done for this chain, or asked within the re-ask window).
pub async fn on_covered_wall(
    state: &AppState,
    scope: &SelfScope,
    url: &str,
    site: &str,
    host: &str,
    servers: &[McpServer],
) -> Option<String> {
    let origin = scope.origin.clone()?;
    let chat = super::human_steps::chat_key(scope)?;
    let mut checked = Vec::with_capacity(servers.len());
    for s in servers {
        checked.push((s.name.clone(), health(state, s).await));
    }
    let (names, health) = pick(&checked);
    if health == Health::Works {
        if !may_follow_up(&scope.kwant_id, &chat, site) {
            tracing::info!(kwant_id = %scope.kwant_id, host, "a follow-up turn already ended on this sign-in wall: not starting another");
            return None;
        }
        let kwant = state.db.get_kwant(&scope.kwant_id).await.ok().flatten()?;
        let prompt = follow_up_prompt(host, &names);
        let job = state.turns.start_job([chat.clone()]);
        let turn_scope = match scope_for(state, &kwant, origin.channel_id.clone(), origin.thread_id.clone(), &job, origin.depth).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(kwant_id = %scope.kwant_id, error = %e, "couldn't start the follow-up turn for a covered sign-in wall");
                return None;
            }
        };
        super::human_steps::spawn_turn(turn_scope, kwant, prompt.clone(), job);
        tracing::info!(kwant_id = %scope.kwant_id, host, tools = ?names, "a turn ended on a sign-in wall it has tools for: follow-up turn started");
        return Some(prompt);
    }
    let since = chrono::Utc::now() - chrono::Duration::hours(super::human_steps::SIGN_IN_REASK_HOURS);
    if state.db.tools_step_asked_since(&scope.kwant_id, site, since).await.unwrap_or(true) {
        return None;
    }
    // the server the step is about: the failing one `pick` named
    let server = names.first().and_then(|n| servers.iter().find(|s| &s.name == n)).or(servers.first())?;
    let reason = failing_reason(&scope.kwant_name, &server.name, host, health);
    super::human_steps::ask_to_fix_tools(state, scope, &reason, url, server).await.then_some(reason)
}

/// C: the line to add to a browser result whose page is `page`, once per turn and site; `None`
/// when the page isn't on a site the bot's tools cover, or this turn was already told.
pub async fn browser_hint(state: &AppState, scope: &SelfScope, page: Option<&Value>) -> Option<String> {
    let url = page?.get("url")?.as_str()?;
    let host = url::Url::parse(url).ok()?.host_str()?.trim_start_matches("www.").to_string();
    let label = covers::site_label(&host)?;
    let turn = scope.turn_id.as_deref()?;
    // Counted before the lookup: one look per turn and site, covered or not
    if !may_hint(turn, &label) {
        return None;
    }
    let servers = covers::for_kwant(&state.db, &scope.kwant_id, &host).await;
    if servers.is_empty() {
        return None;
    }
    // The last test only: a browser call doesn't wait on a server test (untested counts as working)
    let checked: Vec<(String, Health)> =
        servers.iter().map(|s| (s.name.clone(), cached_health(&s.id, None).unwrap_or(Health::Works))).collect();
    let (names, health) = pick(&checked);
    Some(browser_hint_text(&names, &host, health))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_outcome_reads_as_one_health() {
        assert_eq!(health_of(true, false, None), Health::Works);
        assert_eq!(health_of(false, true, Some("HTTP 401")), Health::SignedOut);
        let key = format!("transport error: HTTP 401 Unauthorized ({UNAUTHORIZED_KEY_HINT})");
        assert_eq!(health_of(false, false, Some(&key)), Health::KeyRejected);
        assert_eq!(health_of(false, false, Some("transport error: HTTP 403 Forbidden")), Health::KeyRejected);
        assert_eq!(health_of(false, false, Some("connection refused")), Health::NotConnecting);
        assert_eq!(health_of(false, false, None), Health::NotConnecting);
    }

    #[test]
    fn a_working_server_wins_over_a_failing_one() {
        let n = |s: &str| s.to_string();
        assert_eq!(pick(&[(n("a"), Health::KeyRejected), (n("b"), Health::Works)]), (vec![n("b")], Health::Works));
        assert_eq!(pick(&[(n("a"), Health::KeyRejected), (n("b"), Health::NotConnecting)]), (vec![n("a")], Health::KeyRejected));
        assert_eq!(pick(&[(n("a"), Health::Works), (n("b"), Health::Works)]).0, [n("a"), n("b")]);
    }

    /// The exact lines (impl.md quotes them; Mira's copy check reads the person-facing ones).
    #[test]
    fn the_lines_name_the_tools_and_the_real_problem() {
        let gh = vec!["github".to_string()];
        assert_eq!(
            failing_reason("Architect", "github", "github.com", Health::KeyRejected),
            "Architect has github tools for github.com, but the secret saved for github was rejected. Edit github in Tools and paste a new secret, then press Done."
        );
        assert_eq!(
            failing_reason("Architect", "github", "github.com", Health::SignedOut),
            "Architect has github tools for github.com, but their sign-in has run out. Sign in to github again in Tools, then press Done."
        );
        assert_eq!(
            failing_reason("Architect", "github", "github.com", Health::NotConnecting),
            "Architect has github tools for github.com, but they aren't connecting. Press Test connection on github in Tools to see why, then press Done."
        );
        assert_eq!(browser_hint_text(&gh, "github.com", Health::Works), "You have the `github` tools for github.com; use them rather than the browser.");
        assert_eq!(
            browser_hint_text(&gh, "github.com", Health::KeyRejected),
            "You have the `github` tools for github.com, but they can't connect right now: the secret saved for it was rejected. \
             Tell the person to fix it in Tools rather than asking them to sign in to github.com in the browser."
        );
        assert!(browser_hint_text(&gh, "github.com", Health::SignedOut).contains("right now: their sign-in has run out. Tell"));
        // Tools has no "MCP servers" section, and calls the value a secret, not a key
        for h in [Health::KeyRejected, Health::SignedOut, Health::NotConnecting] {
            for line in [failing_reason("Architect", "github", "github.com", h), browser_hint_text(&gh, "github.com", h)] {
                assert!(!line.contains("MCP servers") && !line.contains(" key"), "{line}");
            }
        }
        let prompt = follow_up_prompt("github.com", &gh);
        assert!(prompt.starts_with("[Your browser stopped at github.com's sign-in page]\nYou have the `github` tools for github.com"), "{prompt}");
        assert!(prompt.contains("request_human_step"), "{prompt}");
        assert!(!failing_reason("Architect", "github", "github.com", Health::KeyRejected).contains("sign in to github.com"));
    }

    /// B never loops: one follow-up per bot, chat and site, until the person writes there again.
    #[test]
    fn a_covered_wall_gets_one_follow_up_per_chat_and_site() {
        let (bot, chat) = ("bot-follow-up-test", "chat-follow-up-test");
        assert!(may_follow_up(bot, chat, "https://github.com/"));
        assert!(!may_follow_up(bot, chat, "https://github.com/"), "the follow-up ended on the same wall");
        assert!(may_follow_up(bot, chat, "https://gitlab.com/"), "another site");
        assert!(may_follow_up(bot, "other-chat-follow-up-test", "https://github.com/"), "another chat");
        forget_chat(bot, chat);
        assert!(may_follow_up(bot, chat, "https://github.com/"), "the person wrote: a new chain");
    }

    #[test]
    fn a_turn_hears_about_a_sites_tools_once() {
        assert!(may_hint("turn-hint-test", "github"));
        assert!(!may_hint("turn-hint-test", "github"));
        assert!(may_hint("turn-hint-test", "notion"));
        assert!(may_hint("turn-hint-test-2", "github"));
    }

    #[test]
    fn an_old_memo_entry_is_forgotten() {
        let map = Mutex::new(HashMap::new());
        assert!(first_time(&map, "k".into(), Duration::from_millis(1)));
        std::thread::sleep(Duration::from_millis(5));
        assert!(first_time(&map, "k".into(), Duration::from_millis(1)));
    }
}
