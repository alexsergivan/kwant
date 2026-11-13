//! Suggest from the person's own words (team-from-words, brief "Server contract"): one drafting
//! call on the space's first ready account, with no tools and a JSON-only answer, checked and
//! kept like any other suggestion. One draft per space at a time; `DELETE` stops it.
//!
//! The model's answer is untrusted text: [`crate::teams::need::parse`] keeps only names, titles,
//! presets, avatars, scope lines, a reason and the instructions, all cleaned and capped, so a
//! drafted team can never carry tools, accounts or routines.

use super::{fail, fail_at, internal, new_suggestion_id, person, store_put, Suggestion, SUGGESTION_MINUTES};
use crate::db::models::{Kwant, LlmProfile};
use crate::db::spaces::SpaceId;
use crate::db::team_templates as tt;
use crate::drivers::engines::{choose_model, known_models, EngineKind, Tier};
use crate::drivers::TurnInput;
use crate::profiles::{FailClass, ProviderFailure};
use crate::server::spaces::Scope;
use crate::server::{profile_turn, AppState};
use crate::teams::discover::{Part, Row};
use crate::teams::need::{self, Draft, NeedError};
use crate::teams::names;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The most a draft may take, retry and account changes included.
pub const DRAFT_SECS: u64 = 90;
/// Accounts a draft tries when one is refused (out of usage, signed out).
const MAX_ACCOUNTS: usize = 3;
/// The team icon of a drafted team (as a team made by hand).
pub const NEED_ICON: &str = "users";

/// What a need suggestion keeps besides its rows: the words, the team line, and each row's drafted
/// instructions and "leaves to others" lines, by row key.
#[derive(Debug, Clone, Default)]
pub struct NeedDraft {
    pub words: String,
    pub description: String,
    pub bots: HashMap<String, NeedBot>,
}

#[derive(Debug, Clone, Default)]
pub struct NeedBot {
    pub instructions: String,
    pub avoids: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// One draft per space

struct Running {
    token: u64,
    abort: tokio::task::AbortHandle,
}

fn running() -> &'static Mutex<HashMap<String, Running>> {
    static RUNNING: OnceLock<Mutex<HashMap<String, Running>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// A slot's key: the server's data folder and the space, so two servers in one process (tests)
/// never share a slot.
fn slot_key(state: &AppState, space: &SpaceId) -> String {
    format!("{}\u{0}{}", state.config.data_dir.display(), space.as_str())
}

/// Holds a space's draft slot; dropping it (the answer went out, or the person's request went
/// away) stops that draft and frees the slot, unless a `DELETE` already did.
struct Slot {
    key: String,
    token: u64,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut m = running().lock().unwrap_or_else(|e| e.into_inner());
        if m.get(&self.key).is_some_and(|r| r.token == self.token) {
            if let Some(r) = m.remove(&self.key) {
                r.abort.abort();
            }
        }
    }
}

/// Stops the draft under `key` ([`slot_key`]), if one runs. True when one did.
fn stop(key: &str) -> bool {
    let mut m = running().lock().unwrap_or_else(|e| e.into_inner());
    match m.remove(key) {
        Some(r) => {
            r.abort.abort();
            true
        }
        None => false,
    }
}

/// `DELETE /api/teams/suggest?space=`: stops the space's running draft and frees its slot. 204,
/// also when nothing runs.
pub async fn cancel(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope) -> Response {
    if let Err(r) = person(&state, &headers).await {
        return r;
    }
    if stop(&slot_key(&state, &space)) {
        tracing::info!(space = %space.as_str(), "the person stopped a team draft");
    }
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------------------------
// The draft

fn no_account() -> Response {
    fail(StatusCode::CONFLICT, "no_ready_account", "no AI account in this space can run right now; add one or sign one in first")
}

async fn limited(state: &AppState, account: &str, until: DateTime<Utc>) -> Response {
    let at = profile_turn::clock_text(state, until).await;
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({
            "error": format!("{account} is out of usage until {at}"),
            "code": "account_limited",
            "account": account,
            "until": profile_turn::rfc3339(until),
        })),
    )
        .into_response()
}

/// Why a draft gave nothing.
#[derive(Debug)]
enum Fail {
    /// Every account tried was out of usage: the one back first.
    Limited(String, DateTime<Utc>),
    /// Every account tried was signed out.
    NoAccount,
    /// The answer was unusable twice, the call failed, or it ran past [`DRAFT_SECS`].
    Failed,
}

/// The suggest route's `{ "need": … }`: checks the words, drafts, answers the suggestion.
pub(super) async fn suggest(state: &AppState, space: &SpaceId, raw: &Value) -> Response {
    let Value::String(raw) = raw else {
        return fail_at(StatusCode::BAD_REQUEST, "bad_request", "need must be text", "need");
    };
    let words = match need::check_need(raw) {
        Ok(w) => w,
        Err(NeedError::TooShort) => {
            return fail_at(StatusCode::UNPROCESSABLE_ENTITY, "need_too_short", format!("say a bit more: at least {} characters", need::MIN_NEED), "need")
        }
        Err(NeedError::TooLong) => {
            return fail_at(StatusCode::UNPROCESSABLE_ENTITY, "need_too_long", format!("that's over {} characters; shorten it a little", need::MAX_NEED), "need")
        }
    };
    let accounts = profile_turn::call_accounts(state, space).await;
    if accounts.ready.is_empty() {
        return match accounts.limited {
            Some((name, until)) => limited(state, &name, until).await,
            None => no_account(),
        };
    }

    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    let key = slot_key(state, space);
    let task = {
        let mut m = running().lock().unwrap_or_else(|e| e.into_inner());
        if m.contains_key(&key) {
            return fail(StatusCode::CONFLICT, "busy", "a team is already being suggested in this space; wait for it or cancel it");
        }
        let (st, w, profiles) = (state.clone(), words.clone(), accounts.ready);
        let task = tokio::spawn(async move { draft(&st, &w, &profiles).await });
        m.insert(key.clone(), Running { token, abort: task.abort_handle() });
        task
    };
    let _slot = Slot { key, token };
    let started = Instant::now();
    let outcome = match task.await {
        Ok(o) => o,
        Err(e) if e.is_cancelled() => return fail(StatusCode::CONFLICT, "cancelled", "the suggestion was stopped"),
        Err(_) => return internal(),
    };
    match outcome {
        Ok(d) => answer(state, space, words, d, started.elapsed().as_millis() as u64).await,
        Err(Fail::Limited(name, until)) => limited(state, &name, until).await,
        Err(Fail::NoAccount) => no_account(),
        Err(Fail::Failed) => fail(StatusCode::BAD_GATEWAY, "draft_failed", "the AI couldn't draft a team this time; try again or put it another way"),
    }
}

/// The draft itself: accounts in order until one answers, one retry on an unusable answer.
async fn draft(state: &AppState, words: &str, profiles: &[LlmProfile]) -> Result<Draft, Fail> {
    let deadline = Instant::now() + Duration::from_secs(DRAFT_SECS);
    let scratch = Scratch::new(state);
    let mut out_until: Option<(String, DateTime<Utc>)> = None;
    let mut signed_out = false;
    let mut other = false;
    for p in profiles.iter().take(MAX_ACCOUNTS) {
        let mut retry: Option<String> = None;
        loop {
            match call(state, p, need::prompt(words, retry.as_deref()), scratch.path(), deadline).await {
                Call::Text(text) => match need::parse(&text) {
                    Ok(d) => {
                        tracing::info!(account = %p.name, bots = d.bots.len(), trimmed = d.trimmed, retried = retry.is_some(), "drafted a team from the person's words");
                        return Ok(d);
                    }
                    Err(why) if retry.is_none() => {
                        tracing::info!(account = %p.name, reason = %why, "a team draft was unusable; asking once more");
                        retry = Some(why);
                    }
                    Err(why) => {
                        tracing::warn!(account = %p.name, reason = %why, "a team draft was unusable twice");
                        return Err(Fail::Failed);
                    }
                },
                Call::Refused(f) => {
                    profile_turn::record_failure(state, p, &f).await;
                    match f.class {
                        FailClass::Quota => {
                            let until = match f.retry_at {
                                Some(t) => Some(t),
                                None => state.db.list_profile_states().await.ok().and_then(|m| m.get(&p.id).and_then(|s| s.limited_until)),
                            }
                            .unwrap_or_else(|| tt::now() + ChronoDuration::hours(1));
                            if out_until.as_ref().is_none_or(|(_, t)| until < *t) {
                                out_until = Some((p.name.clone(), until));
                            }
                        }
                        FailClass::Auth => signed_out = true,
                        FailClass::Transient => other = true,
                    }
                    tracing::info!(account = %p.name, class = %f.class.as_str(), "a team draft's account refused; trying the next");
                    break;
                }
                Call::Failed(why) => {
                    tracing::warn!(account = %p.name, reason = %why, "a team draft failed");
                    return Err(Fail::Failed);
                }
            }
        }
    }
    match (out_until, signed_out, other) {
        (_, _, true) => Err(Fail::Failed),
        (Some((name, until)), _, false) => Err(Fail::Limited(name, until)),
        (None, true, false) => Err(Fail::NoAccount),
        (None, false, false) => Err(Fail::Failed),
    }
}

/// An empty folder a draft runs in (a CLI needs one), removed afterwards.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(state: &AppState) -> Self {
        let dir = state.config.data_dir.join("tmp").join(format!("team-draft-{}", uuid::Uuid::new_v4().simple()));
        let _ = std::fs::create_dir_all(&dir);
        Scratch(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

enum Call {
    Text(String),
    Refused(ProviderFailure),
    Failed(String),
}

/// One no-tools model call on `p`, as a chat turn would pick its engine and model (the Balanced
/// tier), by a stand-in bot that is never saved.
async fn call(state: &AppState, p: &LlmProfile, prompt: String, dir: &std::path::Path, deadline: Instant) -> Call {
    let target_model = p.engine.clone();
    let (key, endpoint) = if p.kind == "system" {
        (crate::server::routes::get_configured_key(&state.db, &target_model).await, crate::server::routes::get_configured_endpoint(&state.db, &target_model).await)
    } else {
        (None, None)
    };
    let (driver, host_engine) = match profile_turn::driver_for(state, Some(p), &target_model, key, endpoint).await {
        Ok(d) => d,
        Err(why) => return Call::Failed(why),
    };
    let runs_on = host_engine.or_else(|| profile_turn::profile_engine(p));
    let custom = p.kind == "custom_api";
    let choice = if custom {
        profile_turn::custom_model(state, p, None, Tier::Balanced).await
    } else {
        runs_on.and_then(|e| choose_model(&known_models(&state.engines, e), None, Tier::Balanced))
    };
    let unlisted_api = !custom && runs_on.is_some_and(|e| e.kind() == EngineKind::Api && state.engines.report(e).models.is_empty());
    let input = TurnInput {
        thread_id: format!("team-draft-{}", uuid::Uuid::new_v4().simple()),
        kwant: Kwant {
            space_id: p.space_id.clone(),
            id: "team-drafter".into(),
            name: "TeamDrafter".into(),
            title: "Team drafter".into(),
            persona_prompt: need::DRAFTER_PERSONA.into(),
            soul: String::new(),
            avatar_sprite: need::DEFAULT_AVATAR.into(),
            driver_type: "cli".into(),
            preferred_model: target_model.clone(),
            temperature: 0.4,
            max_tokens: 4096,
            level: 1,
            turns_completed: 0,
            created_at: None,
        },
        prompt,
        history: Vec::new(),
        model_override: None,
        model: if unlisted_api { None } else { choice.as_ref().and_then(|c| c.run_id()).map(str::to_string) },
        effort_level: None,
        team_context: None,
        workspace_dir: Some(dir.to_path_buf()),
        browser_api_url: None,
        sandbox_id: None,
        teammates: Vec::new(),
        continuity: String::new(),
        skills: String::new(),
        person: Default::default(),
        store: None,
        mcp: None,
        no_tools: true,
        read_only: false,
        delivery: None,
        fresh_eyes: true,
        expects: String::new(),
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
    let out = tokio::time::timeout(left, driver.stream_turn(input, tx)).await;
    drain.abort();
    match out {
        Err(_) => Call::Failed(format!("no answer within {DRAFT_SECS} s")),
        Ok(Err(e)) => Call::Failed(e.to_string()),
        Ok(Ok(o)) => match o.failure {
            Some(f) => Call::Refused(f),
            None if o.notice.is_some() => Call::Failed("the engine stopped with a notice".into()),
            None if o.is_error => Call::Failed("the engine reported an error".into()),
            None => Call::Text(o.text),
        },
    }
}

/// The 200: the rows with free names, kept under a new suggestion id.
async fn answer(state: &AppState, space: &SpaceId, words: String, d: Draft, ms: u64) -> Response {
    let keys = need::row_keys(&d.bots);
    let mut taken = HashSet::new();
    let mut rows = Vec::new();
    let mut kept = HashMap::new();
    for (b, key) in d.bots.iter().zip(keys) {
        let Ok(name) = state.db.free_bot_name(space, &b.name, &taken).await else { return internal() };
        taken.insert(name.to_lowercase());
        rows.push(Row {
            key: key.clone(),
            preset: b.preset.clone(),
            lead: b.lead,
            name,
            title: b.title.clone(),
            avatar_sprite: b.avatar.clone(),
            reason_parts: vec![Part::Text(b.reason.clone())],
            reason: b.reason.clone(),
            owns: b.owns.clone(),
            existing_kwant_id: None,
            existing_name: None,
        });
        kept.insert(key, NeedBot { instructions: b.instructions.clone(), avoids: b.avoids.clone() });
    }
    let wanted = names::slug(&d.team_name).unwrap_or_else(|| "team".to_string());
    let Ok(handle) = state.db.free_team_handle(space, &wanted).await else { return internal() };
    let bots: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
            if let Some(o) = v.as_object_mut() {
                o.insert("instructions".into(), json!(kept.get(&r.key).map(|k| k.instructions.as_str()).unwrap_or("")));
            }
            v
        })
        .collect();
    let s = Suggestion {
        id: new_suggestion_id(),
        space: space.as_str().to_string(),
        team_id: None,
        workspace_path: None,
        repo_url: None,
        handle,
        stack_line: None,
        bots: rows,
        also: Vec::new(),
        fallback: false,
        expires_at: tt::now() + ChronoDuration::minutes(SUGGESTION_MINUTES),
        need: Some(NeedDraft { words: words.clone(), description: d.description, bots: kept }),
    };
    let body = json!({
        "suggestion_id": s.id,
        "expires_at": s.expires_at,
        "team_id": null,
        "handle": s.handle,
        "need": words,
        "facts": [],
        "stack_line": null,
        "bots": bots,
        "also_possible": [],
        "read": { "files": 0, "manifests": 0, "ms": ms },
        "partial": null,
        "limits": [],
        "fallback": false,
        "trimmed": d.trimmed,
    });
    store_put(s);
    Json(body).into_response()
}

/// A drafted bot's persona: its known preset's, else one line from its name and title; the lead
/// also gets the lead line.
pub(super) fn persona(r: &Row) -> String {
    let mut s = match crate::teams::presets::get(&r.preset) {
        Some(p) => p.persona_prompt.trim_end().to_string(),
        None => format!("You are {}, the team's {}.", r.name, r.title),
    };
    if r.lead {
        s.push('\n');
        s.push_str("You lead this team: you turn each request into a small plan, hand each job to the right member, and check it is done before you call it done.");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(preset: &str, lead: bool) -> Row {
        Row {
            key: "k".into(),
            preset: preset.into(),
            lead,
            name: "OrderKeeper".into(),
            title: "Order Keeper".into(),
            avatar_sprite: "data".into(),
            reason_parts: Vec::new(),
            reason: String::new(),
            owns: Vec::new(),
            existing_kwant_id: None,
            existing_name: None,
        }
    }

    #[test]
    fn personas_come_from_known_presets_else_the_name_and_title() {
        assert_eq!(persona(&row("custom", false)), "You are OrderKeeper, the team's Order Keeper.");
        let support = persona(&row("support", false));
        assert_eq!(support, crate::teams::presets::get("support").unwrap().persona_prompt.trim_end());
        assert!(persona(&row("custom", true)).contains("You lead this team"));
    }

    #[test]
    fn one_slot_per_space_and_stop_frees_it() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let task = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(60)).await });
            running().lock().unwrap().insert("spc_slot_test".into(), Running { token: 7, abort: task.abort_handle() });
            {
                // Another request's slot doesn't free this one
                let _other = Slot { key: "spc_slot_test".into(), token: 8 };
            }
            assert!(running().lock().unwrap().contains_key("spc_slot_test"));
            assert!(stop("spc_slot_test"));
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(!stop("spc_slot_test"));
            // The answer going out frees its own slot
            let task = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(60)).await });
            running().lock().unwrap().insert("spc_slot_test".into(), Running { token: 9, abort: task.abort_handle() });
            drop(Slot { key: "spc_slot_test".into(), token: 9 });
            assert!(!running().lock().unwrap().contains_key("spc_slot_test"));
            assert!(task.await.unwrap_err().is_cancelled());
        });
    }
}
