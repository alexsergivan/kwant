//! `/api/app/update`: the Mac app's in-app update (see [`crate::app_update`]). GET is open to
//! bots like other status reads; check and install are person-only (`auth::person_only`).

use super::continuity::{notify, Notification};
use super::AppState;
use crate::app_update::{Refused, State, Status};
use axum::extract::State as AxState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

/// The setting that remembers which version the person was already told about.
pub const NOTIFIED_SETTING: &str = "app_update_notified_version";

/// `GET /api/app/update`: `{supported, current, state, available, progress, error}`.
pub async fn get_status(AxState(state): AxState<AppState>) -> Json<Status> {
    Json(state.app_update.status())
}

/// `POST /api/app/update/check`: 202 with the status; 409 off the Mac app or while busy.
pub async fn check(AxState(state): AxState<AppState>) -> Response {
    reply(state.app_update.check())
}

/// `POST /api/app/update/install`: 202 with the status; 409 off the Mac app, while busy, or with
/// no newer version found. The app relaunches by itself once the new version is in place.
pub async fn install(AxState(state): AxState<AppState>) -> Response {
    reply(state.app_update.install())
}

fn reply(result: Result<Status, Refused>) -> Response {
    match result {
        Ok(status) => (StatusCode::ACCEPTED, Json(status)).into_response(),
        Err(r) => (StatusCode::CONFLICT, Json(serde_json::json!({ "error": r.message() }))).into_response(),
    }
}

/// Tells the person once per version that an update is out, through the usual notifications.
/// Runs for the life of the server; does nothing off the Mac app, where the state never changes.
pub fn spawn_notifier(state: AppState) {
    let mut rx = state.app_update.subscribe();
    tokio::spawn(async move {
        loop {
            let available = {
                let s = rx.borrow_and_update();
                (s.state == State::Available).then(|| s.available.clone()).flatten()
            };
            if let Some(a) = available {
                notify_once(&state, &a.version).await;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    });
}

/// Sends the "new version" notification unless this version was already announced.
pub async fn notify_once(state: &AppState, version: &str) -> bool {
    let told = state.db.get_setting(NOTIFIED_SETTING).await.ok().flatten();
    if told.as_deref() == Some(version) {
        return false;
    }
    if state.db.set_setting(NOTIFIED_SETTING, version).await.is_err() {
        return false;
    }
    notify(
        state,
        Notification {
            kind: "app_update",
            kwant_id: String::new(),
            kwant_name: "kwant".into(),
            channel_id: None,
            title: format!("kwant bots {version} is ready"),
            body: "Open kwant bots to update and restart.".into(),
        },
    )
    .await;
    true
}
