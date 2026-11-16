//! The person's approvals switch: `ask` (an approval card before a bot's tool runs), `smart`
//! (a card only for risky calls; the rest run and are recorded as `auto_allowed`) or `off` (it
//! runs, and is recorded as `auto_allowed`). Set per space, with a per-bot override. Only
//! the person changes it: a request carrying a bot's turn token is refused, and bots' own tools
//! refuse an `approvals` field.

use super::{
    spaces::{active_space, Scope},
    AppState,
};
use crate::db::{
    approval_modes::{valid_mode, ApprovalMode, INHERIT, OFF, SMART},
    spaces::SpaceId,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

fn internal() -> Response {
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

const REFUSAL: &str = "only the person changes approvals";

/// Modes that let a bot run commands without a card: kept away from the built-in password store.
fn relaxes(mode: &str) -> bool {
    mode == OFF || mode == SMART
}

#[derive(Deserialize)]
pub struct ModeBody {
    pub approvals: String,
    /// The person accepts the built-in password store's risk (`passwords::accept_risk`).
    #[serde(default)]
    pub accept_risk: bool,
}

/// A bot's setting as the API shows it, next to its other fields.
pub fn mode_json(m: &ApprovalMode) -> Value {
    json!({"approvals": m.bot, "approvals_effective": m.effective})
}

#[allow(clippy::result_large_err)]
async fn space_json(state: &AppState, space: &SpaceId) -> Result<Value, Response> {
    let mode = state.db.space_approvals(space).await.map_err(|_| internal())?;
    let kwants = state.db.list_kwants(space).await.map_err(|_| internal())?;
    let mut bots = Vec::with_capacity(kwants.len());
    for k in kwants {
        let m = state.db.kwant_approvals_in(space, &k.id).await.map_err(|_| internal())?;
        let mut item = mode_json(&m);
        item["kwant_id"] = json!(k.id);
        bots.push(item);
    }
    Ok(json!({"space_id": space.as_str(), "approvals": mode, "bots": bots}))
}

/// `GET /api/spaces/:id/approvals`: `{space_id, approvals: "ask"|"smart"|"off", bots: [{kwant_id,
/// approvals: "inherit"|"ask"|"smart"|"off", approvals_effective: "ask"|"smart"|"off"}]}`.
pub async fn get_space(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let space = SpaceId::new(&id);
    match state.db.get_space(&space).await {
        Ok(Some(_)) => {}
        Ok(None) => return error(StatusCode::NOT_FOUND, "no such space"),
        Err(_) => return internal(),
    }
    match space_json(&state, &space).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

/// `PUT /api/spaces/:id/approvals {approvals: "ask"|"smart"|"off"}`: the person only.
pub async fn put_space(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ModeBody>,
) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let space = match active_space(&state, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let mode = body.approvals.trim();
    if !valid_mode(mode) {
        return error(StatusCode::BAD_REQUEST, "approvals must be \"ask\", \"smart\" or \"off\"");
    }
    if relaxes(mode) {
        if let Err(r) = super::passwords::accept_risk(&state, &headers, body.accept_risk).await {
            return r;
        }
        if let Some(r) = super::passwords::refuse_approvals_off(&state).await {
            return r;
        }
    }
    if state.db.set_space_approvals(&space, mode).await.is_err() {
        return internal();
    }
    tracing::info!(space = %space, approvals = mode, "the person set the space's approvals");
    match space_json(&state, &space).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

#[allow(clippy::result_large_err)]
async fn kwant_json(state: &AppState, id: &str) -> Result<Value, Response> {
    let Some(space) = state.db.space_of_kwant(id).await.map_err(|_| internal())? else {
        return Err(error(StatusCode::NOT_FOUND, "no such bot"));
    };
    let m = state.db.kwant_approvals_in(&space, id).await.map_err(|_| internal())?;
    let mut v = mode_json(&m);
    v["kwant_id"] = json!(id);
    v["space_approvals"] = json!(m.space);
    Ok(v)
}

/// `GET /api/kwants/:id/approvals`: `{kwant_id, approvals: "inherit"|"ask"|"smart"|"off",
/// approvals_effective, space_approvals}`.
pub async fn get_kwant(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match kwant_json(&state, &id).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

/// `PUT /api/kwants/:id/approvals {approvals: "inherit"|"ask"|"smart"|"off"}`: the person only.
pub async fn put_kwant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ModeBody>,
) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let mode = body.approvals.trim();
    if !(valid_mode(mode) || mode == INHERIT) {
        return error(StatusCode::BAD_REQUEST, "approvals must be \"inherit\", \"ask\", \"smart\" or \"off\"");
    }
    let space = match state.db.space_of_kwant(&id).await {
        Ok(Some(s)) => s,
        Ok(None) => return error(StatusCode::NOT_FOUND, "no such bot"),
        Err(_) => return internal(),
    };
    // `inherit` in a space that is off (or smart) relaxes the bot too
    let turns_off = relaxes(mode)
        || (mode == INHERIT && state.db.space_approvals(&space).await.map_or(true, |m| relaxes(&m)));
    if turns_off {
        if let Err(r) = super::passwords::accept_risk(&state, &headers, body.accept_risk).await {
            return r;
        }
        if let Some(r) = super::passwords::refuse_approvals_off(&state).await {
            return r;
        }
    }
    if state.db.set_kwant_approvals(&id, mode).await.is_err() {
        return internal();
    }
    tracing::info!(kwant = %id, approvals = mode, "the person set a bot's approvals");
    match kwant_json(&state, &id).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub kwant_id: Option<String>,
}

/// `GET /api/approvals?space=&status=&kwant_id=`: the space's approval records, newest first
/// (max 200). `status=auto_allowed` lists what ran while approvals were off. Arguments are
/// masked as in `/api/away`.
pub async fn list(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<ListQuery>) -> Response {
    let status = q.status.filter(|s| !s.is_empty());
    let kwant = q.kwant_id.filter(|k| !k.is_empty());
    let rows = match state.db.list_approvals(&space, status.as_deref(), kwant.as_deref()).await {
        Ok(r) => r,
        Err(_) => return internal(),
    };
    let out: Vec<Value> = rows
        .into_iter()
        .map(|a| {
            let details = serde_json::from_str(&a.parameters).unwrap_or(Value::Null);
            json!({
                "id": a.id,
                "turn_id": a.turn_id,
                "kwant_id": a.kwant_id,
                "tool_name": a.tool_name,
                "status": a.status,
                "answered_via": a.answered_via,
                "created_at": a.created_at,
                "details": super::away::masked_details(details),
                "link": approval_link(a.resume.as_deref()),
            })
        })
        .collect();
    Json(out).into_response()
}

/// 22a: a row's `link`, the chat it happened in, from its stored `resume` in the same
/// `{thread_id, channel_id, direct_kwant_id}` shape as a live card's link (each key present,
/// `null` when not that kind). `null` for a row stored without a chat (before 22a, or never in one).
fn approval_link(resume: Option<&str>) -> Value {
    let Some(chat) = resume.and_then(|r| serde_json::from_str::<Value>(r).ok()) else {
        return Value::Null;
    };
    let field = |k: &str| chat.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let (thread_id, channel_id, direct_kwant_id) = (field("thread_id"), field("channel_id"), field("direct_kwant_id"));
    if thread_id.is_none() && channel_id.is_none() && direct_kwant_id.is_none() {
        return Value::Null;
    }
    json!({"thread_id": thread_id, "channel_id": channel_id, "direct_kwant_id": direct_kwant_id})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rows_link_is_its_stored_chat_in_a_cards_shape() {
        let link = |r: &str| approval_link(Some(r));
        assert_eq!(link(r#"{"thread_id":"t1"}"#), json!({"thread_id": "t1", "channel_id": null, "direct_kwant_id": null}));
        assert_eq!(link(r#"{"channel_id":"c1"}"#), json!({"thread_id": null, "channel_id": "c1", "direct_kwant_id": null}));
        assert_eq!(link(r#"{"direct_kwant_id":"k1"}"#), json!({"thread_id": null, "channel_id": null, "direct_kwant_id": "k1"}));
        // A card's routing extras never show
        assert_eq!(
            link(r#"{"direct_kwant_id":"k1","agent":"claude","effort":"high","purpose":"x"}"#),
            json!({"thread_id": null, "channel_id": null, "direct_kwant_id": "k1"})
        );
        // Stored without a chat, or not one: no link
        assert_eq!(approval_link(None), Value::Null);
        assert_eq!(link("not json"), Value::Null);
        assert_eq!(link(r#"{"purpose":"x"}"#), Value::Null);
        assert_eq!(link(r#"{"thread_id":""}"#), Value::Null);
    }
}
