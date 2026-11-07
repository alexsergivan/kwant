//! `/api/connections/telegram` (next-077 §G1–G3, G5, G6): set up, pair, unpair, switch kinds and
//! approvals, test, disconnect. Person-only: `/api/connections/…` is in `auth::person_only`, and
//! every handler also refuses a bot's turn token. The token is accepted once and never returned.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{api, bridge, copy, db, emit_status, pairing, runner, status_of, AppState, Out};
use crate::sandbox::credentials::StoreError;
use crate::server::mcp_suggest::refuse_agent_tokens;
use crate::server::spaces::Scope;

const REFUSAL: &str = "only the person sets up Telegram";
/// How long Undo can bring back a pairing Unpair ended (A23).
pub const UNDO_SECS: i64 = 10;
const KINDS: &[&str] = &["question", "approval", "human_step", "stuck", "done"];

fn fail(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({"error": {"code": code, "message": message}}))).into_response()
}

fn vault_error(e: StoreError) -> Response {
    match e {
        StoreError::Locked => fail(StatusCode::LOCKED, "vault_locked", "Unlock your saved passwords first, then try again."),
        e => {
            tracing::warn!("telegram: the vault failed: {e}");
            fail(StatusCode::SERVICE_UNAVAILABLE, "vault_unavailable", "Saved passwords can't be reached right now.")
        }
    }
}

fn db_error<E: std::fmt::Display>(e: E) -> Response {
    tracing::warn!("telegram: database: {e}");
    fail(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Something went wrong saving that.")
}

/// The status the GET answers (and what every change answers with).
pub async fn view(state: &AppState, space: &str) -> Value {
    let Ok(Some(link)) = state.db.telegram_link(space).await else {
        let kinds: Value = serde_json::from_str(db::DEFAULT_KINDS).unwrap_or_default();
        return json!({"state": "off", "kinds": kinds, "allow_approvals": false, "status": "offline", "queued": 0});
    };
    let (status, queued) = status_of(state, space);
    let now = crate::clock::now();
    let mut v = json!({
        "state": link.state,
        "kinds": link.kinds_json(),
        "allow_approvals": link.allow_approvals,
        "status": if link.state == db::REJECTED { "offline" } else { status.as_str() },
        "queued": queued,
    });
    let o = v.as_object_mut().expect("an object");
    if let (Some(username), Some(name)) = (&link.bot_username, &link.bot_name) {
        o.insert("bot".into(), json!({"username": username, "name": name}));
    }
    if link.is_paired() {
        o.insert("paired".into(), json!({"name": link.user_name, "username": link.user_username, "since": link.paired_at}));
    }
    if let Some(t) = link.allow_since.filter(|_| link.allow_approvals) {
        o.insert("allow_since".into(), json!(t));
    }
    if let Some(t) = link.last_message_at {
        o.insert("last_message_at".into(), json!(t));
    }
    if link.state == db::PAIRING {
        o.insert("pair_expires_at".into(), json!(link.pair_expires_at));
    }
    if link.pair_locked {
        o.insert("pair_locked".into(), json!(true));
    }
    if let Some(until) = link.prev_until.filter(|t| *t > now && link.state == db::TOKEN_SAVED) {
        o.insert("undo_until".into(), json!(until));
    }
    if let Some(e) = &link.last_error {
        o.insert("error".into(), json!(e));
    }
    if let Some(u) = &link.app_url {
        o.insert("app_url".into(), json!(u));
    }
    o.insert("strangers".into(), json!(link.strangers));
    v
}

pub async fn get(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    Json(view(&state, space.as_str()).await).into_response()
}

#[derive(Deserialize)]
pub struct TokenBody {
    pub token: String,
    /// Optional: where kwant is open (https only), for the Open in kwant buttons.
    #[serde(default)]
    pub app_url: Option<String>,
}

/// `https://host[:port]` (no path, query, user or password), or `None`.
pub fn clean_app_url(raw: &str) -> Option<String> {
    let u = url::Url::parse(raw.trim()).ok()?;
    let plain = u.scheme() == "https" && u.username().is_empty() && u.password().is_none() && u.query().is_none() && u.fragment().is_none() && (u.path() == "/" || u.path().is_empty());
    let host = u.host_str().filter(|h| !h.is_empty())?;
    if !plain {
        return None;
    }
    Some(match u.port() {
        Some(p) => format!("https://{host}:{p}"),
        None => format!("https://{host}"),
    })
}

/// PUT `…/token`: checks the token with Telegram (getMe), saves it to the vault and starts
/// listening. The same bot again (a replaced token) keeps its pairing; another bot starts over.
pub async fn put_token(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope, Json(body): Json<TokenBody>) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    let app_url = match body.app_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => match clean_app_url(raw) {
            Some(u) => Some(u),
            None => return fail(StatusCode::BAD_REQUEST, "app_url_not_https", "The kwant address must start with https:// and have no path."),
        },
        None => None,
    };
    let Some(token) = api::clean_token(&body.token) else {
        return fail(StatusCode::BAD_REQUEST, "token_shape", "That doesn't look like a bot token from BotFather.");
    };
    let bot = match api::Client::new(token.clone()).get_me().await {
        Ok(b) => b,
        Err(api::TgError::Unauthorized) => return fail(StatusCode::BAD_REQUEST, "token_rejected", "Telegram didn't accept that token. Copy it again from BotFather."),
        Err(api::TgError::TooMany { .. }) => return fail(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "Telegram asked kwant to slow down. Try again in a minute."),
        Err(_) => return fail(StatusCode::BAD_GATEWAY, "telegram_unreachable", "kwant couldn't reach Telegram. Check the connection and try again."),
    };
    if let Err(e) = super::save_token(&state, &space, &token).await {
        return vault_error(e);
    }
    let before = state.db.telegram_link(&space).await.ok().flatten();
    let same_bot = before.as_ref().is_some_and(|l| l.bot_id == Some(bot.id));
    let r = if same_bot {
        let l = before.as_ref().expect("checked");
        let next = if l.user_id.is_some() && l.chat_id.is_some() { db::PAIRED } else { db::TOKEN_SAVED };
        state.db.telegram_set_state(&space, next, None).await
    } else {
        // Telegram keeps a token for each bot; the old bot's token is no longer kwant's to keep
        state.db.telegram_save_bot(&space, bot.id, &bot.username, &bot.name).await
    };
    if let Err(e) = r {
        return db_error(e);
    }
    if app_url.is_some() {
        let _ = state.db.telegram_set_app_url(&space, app_url.as_deref()).await;
    }
    super::ensure_running(&state).await;
    super::start(&state, &space, token);
    emit_status(&state, &space);
    Json(view(&state, &space).await).into_response()
}

/// POST `…/pair`: a fresh one-use code for 10 minutes; any earlier code stops working.
pub async fn pair(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    let Ok(Some(link)) = state.db.telegram_link(&space).await else {
        return fail(StatusCode::CONFLICT, "no_token", "Add your bot's token first.");
    };
    let Some(username) = link.bot_username.clone() else {
        return fail(StatusCode::CONFLICT, "no_token", "Add your bot's token first.");
    };
    if link.is_paired() {
        return fail(StatusCode::CONFLICT, "already_paired", "Unpair first to pair another account.");
    }
    if runner(&state, &space).is_none() {
        match super::load_token(&state, &space).await {
            Ok(Some(t)) => super::start(&state, &space, t),
            Ok(None) => return fail(StatusCode::CONFLICT, "no_token", "Add your bot's token first."),
            Err(e) => return vault_error(e),
        }
    }
    let (code, hash, expires_at) = pairing::new_code();
    match state.db.telegram_set_code(&space, &hash, expires_at).await {
        Ok(true) => {}
        Ok(false) => return fail(StatusCode::CONFLICT, "not_ready", "Telegram isn't ready to pair. Check the bot token."),
        Err(e) => return db_error(e),
    }
    emit_status(&state, &space);
    let link = pairing::link(&username, &code);
    let qr = pairing::qr_svg(&link);
    Json(json!({"link": link, "expires_at": expires_at, "qr_svg": qr})).into_response()
}

/// POST `…/unpair`: ends the pairing; Undo brings it back for 10 s. The token stays (A23).
pub async fn unpair(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    let until = crate::clock::now() + chrono::Duration::seconds(UNDO_SECS);
    match state.db.telegram_unpair(&space, Some(until)).await {
        Ok(Some(_)) => {}
        Ok(None) => return fail(StatusCode::CONFLICT, "not_paired", "Telegram isn't paired."),
        Err(e) => return db_error(e),
    }
    emit_status(&state, &space);
    Json(view(&state, &space).await).into_response()
}

/// POST `…/unpair/undo`, within 10 s. Allow approvals stays off.
pub async fn undo_unpair(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    match state.db.telegram_undo_unpair(&space).await {
        Ok(true) => {}
        Ok(false) => return fail(StatusCode::CONFLICT, "undo_expired", "It's too late to undo. Pair again instead."),
        Err(e) => return db_error(e),
    }
    emit_status(&state, &space);
    Json(view(&state, &space).await).into_response()
}

#[derive(Deserialize)]
pub struct PatchBody {
    #[serde(default)]
    pub kinds: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub allow_approvals: Option<bool>,
    /// A string sets it (https only); `null` clears it; absent leaves it.
    #[serde(default, deserialize_with = "some_value")]
    pub app_url: Option<Value>,
}

fn some_value<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

/// PATCH: which needs-you kinds go to Telegram, the approvals switch, and kwant's address.
pub async fn patch(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope, Json(body): Json<PatchBody>) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    let Ok(Some(link)) = state.db.telegram_link(&space).await else {
        return fail(StatusCode::CONFLICT, "no_token", "Add your bot's token first.");
    };
    // Check everything before changing anything
    if let Some(k) = &body.kinds {
        if let Some((bad, _)) = k.iter().find(|(name, on)| !KINDS.contains(&name.as_str()) || !on.is_boolean()) {
            return fail(StatusCode::BAD_REQUEST, "bad_kind", &format!("Unknown alert kind: {}", bad.chars().take(40).collect::<String>()));
        }
    }
    let app_url = match &body.app_url {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) if s.trim().is_empty() => Some(None),
        Some(Value::String(s)) => match clean_app_url(s) {
            Some(u) => Some(Some(u)),
            None => return fail(StatusCode::BAD_REQUEST, "app_url_not_https", "The kwant address must start with https:// and have no path."),
        },
        Some(_) => return fail(StatusCode::BAD_REQUEST, "app_url_not_https", "The kwant address must start with https:// and have no path."),
    };
    if body.allow_approvals == Some(true) && !link.is_paired() {
        return fail(StatusCode::CONFLICT, "not_paired", "Pair Telegram first.");
    }
    if let Some(k) = &body.kinds {
        let mut kinds = link.kinds_json();
        for (name, on) in k {
            kinds[name] = on.clone();
        }
        if let Err(e) = state.db.telegram_set_kinds(&space, &kinds).await {
            return db_error(e);
        }
    }
    if let Some(u) = app_url {
        if let Err(e) = state.db.telegram_set_app_url(&space, u.as_deref()).await {
            return db_error(e);
        }
    }
    match body.allow_approvals {
        Some(true) => {
            if !state.db.telegram_set_allow(&space, true).await.unwrap_or(false) {
                return fail(StatusCode::CONFLICT, "not_paired", "Pair Telegram first.");
            }
        }
        Some(false) => {
            let _ = state.db.telegram_set_allow(&space, false).await;
            bridge::drop_allow_buttons(&state, &space).await;
        }
        None => {}
    }
    emit_status(&state, &space);
    Json(view(&state, &space).await).into_response()
}

/// POST `…/test`: "Test from kwant. If you see this, alerts reach you." (A24)
pub async fn test(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    let Some(chat) = state.db.telegram_link(&space).await.ok().flatten().filter(|l| l.is_paired()).and_then(|l| l.chat_id) else {
        return fail(StatusCode::CONFLICT, "not_paired", "Pair Telegram first.");
    };
    let Some(r) = runner(&state, &space) else {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "offline", "kwant isn't connected to Telegram right now.");
    };
    r.enqueue(chat, Out::Send { parts: vec![super::text::escape(copy::TEST)], markup: None, reply_to: None, ask_id: None, message_id: None });
    (StatusCode::ACCEPTED, Json(json!({"queued": true}))).into_response()
}

/// DELETE: Disconnect. Stops listening, deletes the token from the vault and forgets the link.
/// Nothing is forgotten when the vault can't delete the token.
pub async fn delete(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = space.as_str().to_string();
    if let Err(e) = super::forget_token(&state, &space).await {
        return vault_error(e);
    }
    super::stop(&state, &space);
    if let Err(e) = state.db.telegram_delete(&space).await {
        return db_error(e);
    }
    emit_status(&state, &space);
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_url_is_an_https_origin_only() {
        assert_eq!(clean_app_url("https://kwant.example.com").as_deref(), Some("https://kwant.example.com"));
        assert_eq!(clean_app_url(" https://kwant.example.com:8443/ ").as_deref(), Some("https://kwant.example.com:8443"));
        for bad in ["http://kwant.example.com", "https://user:pw@kwant.example.com", "https://kwant.example.com/x", "https://kwant.example.com?a=1", "javascript:alert(1)", "kwant.example.com", ""] {
            assert_eq!(clean_app_url(bad), None, "{bad}");
        }
    }
}
