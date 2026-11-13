//! `/api/connections/bot-email` (spec §2, ENG-2/3/5/14): providers, status, the two-phase test
//! (sign in, send, receive; then the reply check), ask settings, per-bot on/off, delete.
//! Person-only: `/api/connections/…` is in `auth::person_only`, and every handler also refuses a
//! bot's turn token. The password is accepted once, saved to the vault, and never returned.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    forget_password, hour_ago, load_config, random_part, save_password, AppState,
    REPLY_CHECK_MINUTES,
};
use crate::db::bot_email::MailTag;
use crate::mail::config::{
    bot_slug, new_tag, provider, MailConfig, Mailbox, Server, ASK_DELAYS, PROVIDERS, SAME_MAILBOX,
};
use crate::mail::imap_smtp::ImapSmtp;
use crate::mail::inbox;
use crate::mail::outgoing::{self, Draft};
use crate::mail::MailProvider;
use crate::sandbox::credentials::StoreError;
use crate::server::mcp_suggest::refuse_agent_tokens;
use crate::server::spaces::Scope;

const REFUSAL: &str = "only the person sets up bot email";
/// The receive check waits this long for the test mail (spec ENG-3 `receive_timeout`).
pub const RECEIVE_SECS: u64 = 60;
const RECEIVE_POLL: Duration = Duration::from_secs(3);

/// The running or last setup test per (data dir, space).
static TESTS: LazyLock<Mutex<HashMap<(PathBuf, String), Value>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn fail(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"error": {"code": code, "message": message}})),
    )
        .into_response()
}

fn vault_error(e: StoreError) -> Response {
    match e {
        StoreError::Locked => fail(
            StatusCode::LOCKED,
            "vault_locked",
            "Unlock your saved passwords first, then try again.",
        ),
        e => {
            tracing::warn!("bot email: the vault failed: {e}");
            fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "vault_unavailable",
                "Saved passwords can't be reached right now.",
            )
        }
    }
}

fn db_error<E: std::fmt::Display>(e: E) -> Response {
    tracing::warn!("bot email: database: {e}");
    fail(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        "Something went wrong saving that.",
    )
}

fn invalid(msg: String) -> Response {
    if msg == SAME_MAILBOX {
        return fail(StatusCode::BAD_REQUEST, "same_mailbox", &msg);
    }
    fail(StatusCode::BAD_REQUEST, "invalid", &msg)
}

fn test_key(state: &AppState, space: &str) -> (PathBuf, String) {
    (state.config.data_dir.clone(), space.to_string())
}

fn test_status(state: &AppState, space: &str) -> Option<Value> {
    TESTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&test_key(state, space))
        .cloned()
}

fn set_test(state: &AppState, space: &str, v: Value) {
    TESTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(test_key(state, space), v);
}

/// GET `…/providers`: the setup cards, from code (phase 3 adds one entry).
pub async fn providers(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    Json(json!({"providers": PROVIDERS, "ask_delays": ASK_DELAYS})).into_response()
}

/// What GET answers: never the password.
pub async fn view(state: &AppState, space: &str) -> Value {
    let Some((row, cfg)) = load_config(state, space).await else {
        let mut v = json!({"state": "off"});
        if let Some(t) = test_status(state, space) {
            v["test"] = t;
        }
        return v;
    };
    let sent = state
        .db
        .bot_email_count(space, "out", None, hour_ago())
        .await
        .unwrap_or(0);
    let mut checks: Value = serde_json::from_str(&row.checks).unwrap_or_else(|_| json!({}));
    let waiting = state
        .db
        .bot_email_open_tags(space, "test")
        .await
        .map(|t| !t.is_empty())
        .unwrap_or(false);
    if checks["reply"]["status"] == "waiting" && !waiting {
        checks["reply"] = json!({"status": "fail", "code": "timeout"});
    }
    let attention = match (row.last_error_at, row.last_ok_at) {
        (Some(e), Some(ok)) => e > ok,
        (Some(_), None) => true,
        _ => false,
    };
    let mut v = json!({
        "state": "saved",
        "config": cfg,
        "checks": checks,
        "health": {
            "needs_attention": attention,
            "last_check_at": row.last_check_at,
            "last_ok_at": row.last_ok_at,
            "last_error_at": row.last_error_at,
            "last_error": row.last_error,
            "last_error_code": row.last_error_code,
            "sends_this_hour": sent,
            "sends_per_hour": cfg.sends_per_hour,
        },
    });
    if let Some(t) = test_status(state, space) {
        v["test"] = t;
    }
    v
}

pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    Json(view(&state, space.as_str()).await).into_response()
}

#[derive(Deserialize)]
pub struct TestBody {
    pub provider: String,
    #[serde(default)]
    pub mailbox: Mailbox,
    pub address: String,
    #[serde(default)]
    pub username: String,
    /// Empty: keep the saved one (re-testing an unchanged mailbox).
    #[serde(default)]
    pub password: String,
    pub send_to: Vec<String>,
    /// "Other" only; presets use their own.
    #[serde(default)]
    pub imap: Option<Server>,
    #[serde(default)]
    pub smtp: Option<Server>,
    #[serde(default)]
    pub authserv: Vec<String>,
    #[serde(default)]
    pub only_for_bots: bool,
    #[serde(default)]
    pub accept_risk: bool,
}

/// The pasted password is wiped from memory when the body goes, on every path.
impl Drop for TestBody {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.password);
    }
}

impl TestBody {
    fn config(&self) -> Result<MailConfig, String> {
        let p = provider(&self.provider).ok_or("unknown provider")?;
        let mut c = MailConfig::preset(p, &self.address, self.send_to.clone());
        c.mailbox = self.mailbox;
        c.username = self.username.clone();
        c.only_for_bots = self.only_for_bots;
        if p.id == "other" || p.local_only {
            if let Some(s) = &self.imap {
                c.imap = s.clone();
            }
            if let Some(s) = &self.smtp {
                c.smtp = s.clone();
            }
        }
        if p.id == "other" {
            c.authserv = self.authserv.clone();
        }
        c.settle()
    }
}

/// POST `…/test`: checks 1–3 in the background (GET `…/test` or GET answers the progress), then
/// saves the mailbox and starts the reply check.
pub async fn start_test(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    Json(body): Json<TestBody>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    if state.credentials.is_locked().await {
        return vault_error(StoreError::Locked);
    }
    if let Err(r) = super::super::passwords::accept_risk(&state, &headers, body.accept_risk).await {
        return r;
    }
    if let Some(r) = super::super::passwords::refuse_local_save(&state).await {
        return r;
    }
    let cfg = match body.config() {
        Ok(c) => c,
        Err(e) => return invalid(e),
    };
    let password = if body.password.trim().is_empty() {
        match super::load_password(&state, &space).await {
            Ok(Some(p)) => p,
            Ok(None) => {
                return fail(
                    StatusCode::BAD_REQUEST,
                    "invalid",
                    "Enter the mailbox's app password.",
                )
            }
            Err(e) => return vault_error(e),
        }
    } else {
        zeroize::Zeroizing::new(cfg.clean_password(&body.password))
    };
    if test_status(&state, &space).is_some_and(|t| t["state"] == "running") {
        return fail(
            StatusCode::CONFLICT,
            "test_running",
            "A test is already running.",
        );
    }
    let started = json!({"state": "running", "checks": {"sign_in": "running", "send": "pending", "receive": "pending"}});
    set_test(&state, &space, started.clone());
    let st = state.clone();
    tokio::spawn(async move { run_test(st, space, cfg, password).await });
    (StatusCode::ACCEPTED, Json(json!({"test": started}))).into_response()
}

pub async fn get_test(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    Json(json!({"test": test_status(&state, space.as_str())})).into_response()
}

fn check_fail(
    state: &AppState,
    space: &str,
    checks: &mut Value,
    which: &str,
    code: &str,
    message: &str,
) {
    checks[which] = json!("fail");
    set_test(
        state,
        space,
        json!({"state": "failed", "checks": checks, "failed": which, "error": {"code": code, "message": message}}),
    );
}

async fn run_test(
    state: AppState,
    space: String,
    cfg: MailConfig,
    password: zeroize::Zeroizing<String>,
) {
    let provider = ImapSmtp::new(cfg.clone(), &password);
    let mut checks = json!({"sign_in": "running", "send": "pending", "receive": "pending"});
    // 1. Sign in
    if let Err(e) = provider.check().await {
        return check_fail(&state, &space, &mut checks, "sign_in", e.code, &e.message);
    }
    checks["sign_in"] = json!("pass");
    checks["send"] = json!("running");
    set_test(
        &state,
        &space,
        json!({"state": "running", "checks": checks}),
    );
    // 2. Send M0 to the person and to a fresh tag of the bot mailbox (read back in 3)
    let tag = new_tag(&bot_slug("test"));
    let address = match provider.new_address(&tag).await {
        Ok(a) => a,
        Err(e) => return check_fail(&state, &space, &mut checks, "send", e.code, &e.message),
    };
    let started = crate::clock::now();
    let message_id = format!("t_{}@kwant", random_part(16));
    let d = Draft {
        from_name: "kwant".into(),
        from: address.clone(),
        to: vec![cfg.send_to[0].clone(), address.clone()],
        reply_to: Some(address.clone()),
        subject: outgoing::TEST_SUBJECT.into(),
        text: outgoing::test_text(&cfg.address),
        message_id: message_id.clone(),
        in_reply_to: None,
    };
    let mail = match outgoing::build(&d, &cfg.send_to, &provider.own_addresses()) {
        Ok(m) => m,
        Err(e) => return check_fail(&state, &space, &mut checks, "send", "send_failed", &e),
    };
    if let Err(e) = provider.send(&mail).await {
        return check_fail(&state, &space, &mut checks, "send", e.code, &e.message);
    }
    checks["send"] = json!("pass");
    checks["receive"] = json!("running");
    set_test(
        &state,
        &space,
        json!({"state": "running", "checks": checks}),
    );
    // 3. Read it back, without touching anything else
    let deadline = tokio::time::Instant::now() + Duration::from_secs(RECEIVE_SECS);
    let addresses = provider.tagged().then(|| vec![address.clone()]);
    let mut got = None;
    while got.is_none() {
        let skip = state.db.bot_email_seen(&space).await.unwrap_or_default();
        match inbox::poll(&provider, addresses.clone(), started, &skip).await {
            Ok(list) => {
                got = list
                    .into_iter()
                    .find(|m| {
                        m.parsed.as_ref().and_then(|p| p.message_id.as_deref())
                            == Some(message_id.as_str())
                    })
                    .map(|m| m.key)
            }
            Err(e) => {
                return check_fail(&state, &space, &mut checks, "receive", e.code, &e.message)
            }
        }
        if got.is_none() {
            if tokio::time::Instant::now() >= deadline {
                return check_fail(
                    &state,
                    &space,
                    &mut checks,
                    "receive",
                    "receive_timeout",
                    "The test email didn't arrive in 60 seconds.",
                );
            }
            tokio::time::sleep(RECEIVE_POLL).await;
        }
    }
    checks["receive"] = json!("pass");
    // Saved after 1–3 (ENG-2). Ask settings survive a re-test of the same mailbox and send list
    let mut cfg = cfg;
    let old = load_config(&state, &space).await;
    if let Some((_, o)) = &old {
        if o.address == cfg.address
            && o.send_to == cfg.send_to
            && o.mailbox == cfg.mailbox
            && o.provider == cfg.provider
        {
            cfg.asks = o.asks;
            cfg.asks_since = o.asks_since;
            cfg.reply_checked = o.reply_checked;
            cfg.ask_delay_minutes = o.ask_delay_minutes;
        }
    }
    if let Err(e) = save_password(&state, &space, &cfg, &password).await {
        tracing::warn!(space, "bot email: the password wasn't saved: {e}");
        return check_fail(
            &state,
            &space,
            &mut checks,
            "receive",
            "vault_unavailable",
            "The password couldn't be saved.",
        );
    }
    let reply = if cfg.reply_checked {
        json!({"status": "pass"})
    } else if cfg.mailbox == Mailbox::Own {
        json!({"status": "skipped", "code": "own_mailbox"})
    } else {
        json!({"status": "waiting"})
    };
    let row_checks = json!({"sign_in": "pass", "send": "pass", "receive": "pass", "reply": reply});
    if let Err(e) = state
        .db
        .bot_email_save(
            &space,
            &serde_json::to_string(&cfg).unwrap_or_default(),
            &row_checks.to_string(),
        )
        .await
    {
        tracing::warn!(space, "bot email: the setup wasn't saved: {e}");
        return check_fail(
            &state,
            &space,
            &mut checks,
            "receive",
            "internal",
            "The setup couldn't be saved.",
        );
    }
    let _ = state
        .db
        .bot_email_mark_seen(&space, got.as_deref().unwrap_or_default(), "test")
        .await;
    let _ = state
        .db
        .bot_email_log(
            &space,
            "out",
            None,
            Some(&tag),
            None,
            Some(&crate::mail::safety::base_address(&cfg.send_to[0])),
            "test",
            None,
        )
        .await;
    let _ = state.db.bot_email_health(&space, None).await;
    if reply["status"] == "waiting" {
        let now = crate::clock::now();
        let row = MailTag {
            tag: tag.clone(),
            space_id: space.clone(),
            kwant_id: None,
            kind: "test".into(),
            address,
            site: None,
            question_id: None,
            message_id: Some(message_id),
            chat_message_id: None,
            created_at: started,
            expires_at: now + chrono::Duration::minutes(REPLY_CHECK_MINUTES),
            used_at: None,
        };
        let _ = state.db.bot_email_add_tag(&row).await;
    }
    checks["reply"] = reply["status"].clone();
    set_test(&state, &space, json!({"state": "saved", "checks": checks}));
}

/// POST `…/reply-check`: sends the reply-check mail again and waits 10 more minutes.
pub async fn reply_check(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    let r = match super::ready(&state, &space).await {
        Ok(r) => r,
        Err(super::Unready::Locked) => return vault_error(StoreError::Locked),
        Err(u) => return fail(StatusCode::CONFLICT, u.code(), "Set up bot email first."),
    };
    if r.cfg.mailbox == Mailbox::Own {
        return fail(
            StatusCode::CONFLICT,
            "own_mailbox",
            "Email asks need a mailbox just for the bots.",
        );
    }
    let tag = new_tag(&bot_slug("test"));
    let address = match r.provider.new_address(&tag).await {
        Ok(a) => a,
        Err(e) => return fail(StatusCode::BAD_GATEWAY, e.code, &e.message),
    };
    let now = crate::clock::now();
    let message_id = format!("t_{}@kwant", random_part(16));
    let d = Draft {
        from_name: "kwant".into(),
        from: address.clone(),
        to: vec![r.cfg.send_to[0].clone()],
        reply_to: Some(address.clone()),
        subject: outgoing::TEST_SUBJECT.into(),
        text: outgoing::test_text(&r.cfg.address),
        message_id: message_id.clone(),
        in_reply_to: None,
    };
    if let Err(e) = super::send(&state, &r, &d, None, Some(&tag), None, "test").await {
        return fail(StatusCode::BAD_GATEWAY, "send_failed", &e);
    }
    let row = MailTag {
        tag,
        space_id: space.clone(),
        kwant_id: None,
        kind: "test".into(),
        address,
        site: None,
        question_id: None,
        message_id: Some(message_id),
        chat_message_id: None,
        created_at: now,
        expires_at: now + chrono::Duration::minutes(REPLY_CHECK_MINUTES),
        used_at: None,
    };
    if let Err(e) = state.db.bot_email_add_tag(&row).await {
        return db_error(e);
    }
    super::set_reply_check(&state, &space, json!({"status": "waiting"})).await;
    Json(view(&state, &space).await).into_response()
}

/// POST `…/reply-check/skip`: stop waiting; email asks stay off.
pub async fn skip_reply_check(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    if load_config(&state, &space).await.is_none() {
        return fail(
            StatusCode::NOT_FOUND,
            "not_set_up",
            "Bot email isn't set up.",
        );
    }
    for t in state
        .db
        .bot_email_open_tags(&space, "test")
        .await
        .unwrap_or_default()
    {
        let _ = state.db.bot_email_use_tag(&t.tag).await;
    }
    super::set_reply_check(&state, &space, json!({"status": "skipped"})).await;
    Json(view(&state, &space).await).into_response()
}

#[derive(Deserialize)]
pub struct PatchBody {
    #[serde(default)]
    pub asks: Option<bool>,
    #[serde(default)]
    pub ask_delay_minutes: Option<u32>,
    #[serde(default)]
    pub send_to: Option<Vec<String>>,
}

/// PATCH: email asks on/off, their delay, and the send list. A changed send list needs the
/// reply check again before asks go out.
pub async fn patch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    Json(body): Json<PatchBody>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    let Some((_, mut cfg)) = load_config(&state, &space).await else {
        return fail(
            StatusCode::NOT_FOUND,
            "not_set_up",
            "Bot email isn't set up.",
        );
    };
    if let Some(list) = body.send_to {
        let list: Vec<String> = list
            .iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        if list != cfg.send_to {
            cfg.send_to = list;
            cfg.reply_checked = false;
            cfg.asks = false;
        }
    }
    if let Some(d) = body.ask_delay_minutes {
        cfg.ask_delay_minutes = d;
    }
    if let Some(on) = body.asks {
        if on && !cfg.asks {
            cfg.asks_since = Some(crate::clock::now());
        }
        cfg.asks = on;
    }
    if let Err(e) = cfg.validate() {
        return invalid(e);
    }
    match state
        .db
        .bot_email_set_config(&space, &serde_json::to_string(&cfg).unwrap_or_default())
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return fail(
                StatusCode::NOT_FOUND,
                "not_set_up",
                "Bot email isn't set up.",
            )
        }
        Err(e) => return db_error(e),
    }
    if !cfg.reply_checked {
        super::set_reply_check(
            &state,
            &space,
            json!({"status": "fail", "code": "send_list_changed"}),
        )
        .await;
    }
    Json(view(&state, &space).await).into_response()
}

/// DELETE: forgets the password, the setup, the tags and the mail log of this space.
pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    if let Err(e) = forget_password(&state, &space).await {
        return vault_error(e);
    }
    if let Err(e) = state.db.bot_email_delete(&space).await {
        return db_error(e);
    }
    TESTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&test_key(&state, &space));
    Json(json!({"state": "off"})).into_response()
}

async fn kwant_in_space(state: &AppState, space: &str, kwant: &str) -> bool {
    matches!(state.db.space_of_kwant(kwant).await, Ok(Some(s)) if s.as_str() == space)
}

/// GET `…/bots/{kwant}`: email on/off and the last sign-up address (spec §3, ENG-5).
pub async fn get_bot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    Path(kwant): Path<String>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    if !kwant_in_space(&state, space.as_str(), &kwant).await {
        return fail(StatusCode::NOT_FOUND, "not_found", "No such bot here.");
    }
    let (enabled, last) = state.db.bot_email_bot(&kwant).await.unwrap_or((true, None));
    let set_up = load_config(&state, space.as_str())
        .await
        .map(|(_, c)| (c.address, c.plus_addresses));
    let pattern = set_up.as_ref().map(|(a, plus)| {
        if *plus {
            crate::mail::safety::with_tag(a, "kwant-<bot>-·····")
        } else {
            a.clone()
        }
    });
    Json(json!({"set_up": set_up.is_some(), "enabled": enabled, "last_address": last, "address_pattern": pattern})).into_response()
}

#[derive(Deserialize)]
pub struct BotBody {
    pub enabled: bool,
}

pub async fn put_bot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    Path(kwant): Path<String>,
    Json(body): Json<BotBody>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    if !kwant_in_space(&state, space.as_str(), &kwant).await {
        return fail(StatusCode::NOT_FOUND, "not_found", "No such bot here.");
    }
    if let Err(e) = state.db.bot_email_set_enabled(&kwant, body.enabled).await {
        return db_error(e);
    }
    let (enabled, last) = state
        .db
        .bot_email_bot(&kwant)
        .await
        .unwrap_or((body.enabled, None));
    Json(json!({"enabled": enabled, "last_address": last})).into_response()
}

/// GET `…/questions/{id}`: when an ask went by email and the replies that weren't used (ENG-10).
pub async fn question(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let Ok(Some(q)) = state.db.get_question(&id).await else {
        return fail(StatusCode::NOT_FOUND, "not_found", "No such question.");
    };
    if !kwant_in_space(&state, space.as_str(), &q.kwant_id).await {
        return fail(StatusCode::NOT_FOUND, "not_found", "No such question.");
    }
    let tag = state
        .db
        .bot_email_tag_for_question(&id)
        .await
        .ok()
        .flatten();
    let refused = state.db.bot_email_refused(&id).await.unwrap_or_default();
    let refused: Vec<Value> = refused
        .into_iter()
        .map(|r| json!({"reason": r.reason, "from": r.counterpart, "at": r.created_at}))
        .collect();
    Json(json!({"sent_at": tag.map(|t| t.created_at), "refused": refused})).into_response()
}
