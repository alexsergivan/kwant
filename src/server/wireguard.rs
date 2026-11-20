//! Settings → Browser → WireGuard: paste a WireGuard config and kwant keeps it for the
//! `wireproxy` helper (see `sandbox::wireguard`). The routes are person-only, and no response
//! ever carries a key: a read returns a summary, a save returns the same summary.
use super::AppState;
use crate::sandbox::{browser_proxy, wireguard};
use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};

fn view(state: &AppState) -> Value {
    let proxy = browser_proxy::BrowserProxy::from_env().ok().flatten();
    let uses_helper = proxy.as_ref().is_some_and(|p| p.url == wireguard::HELPER_PROXY_URL);
    let saved = match wireguard::summary(&state.config.data_dir) {
        None => Value::Null,
        Some(Ok(s)) => serde_json::to_value(s).unwrap_or(Value::Null),
        Some(Err(e)) => json!({ "error": e }),
    };
    json!({
        "configured": !saved.is_null(),
        "wireguard": saved,
        "browser_uses_helper": uses_helper,
        "helper_proxy": wireguard::HELPER_PROXY_URL,
        "note": "Use a WireGuard connection made for kwant. A router hands one connection to one device at a time.",
    })
}

/// `GET /api/settings/wireguard`
pub async fn get_wireguard(State(state): State<AppState>) -> Json<Value> {
    Json(view(&state))
}

#[derive(Deserialize)]
pub struct Payload {
    pub config: String,
}

/// `PUT /api/settings/wireguard` with `{"config":"<wg-quick text>"}`. 400 with the reason (never
/// the value) when the text is not a usable config.
pub async fn put_wireguard(
    State(state): State<AppState>,
    payload: Result<Json<Payload>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let bad = |m: String| (StatusCode::BAD_REQUEST, Json(json!({ "error": m })));
    let Json(p) = payload.map_err(|_| bad("Send {\"config\": \"<your WireGuard config>\"}".into()))?;
    wireguard::save(&state.config.data_dir, &p.config).map_err(bad)?;
    Ok(Json(view(&state)))
}

/// `DELETE /api/settings/wireguard`
pub async fn delete_wireguard(State(state): State<AppState>) -> Json<Value> {
    wireguard::remove(&state.config.data_dir);
    Json(view(&state))
}
