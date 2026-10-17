//! "Sign in with Claude" for the Claude engine: `POST /api/engines/claude/login` runs Claude
//! Code's own `auth login --claudeai` on the config dir the engine itself uses. The CLI prints a
//! claude.ai link; claude.ai then shows a code the user pastes back, and the CLI keeps the login.
//! kwnt never reads, stores or logs the token, the code or the link's state.
//!
//! Also `GET`/`DELETE /api/engines/claude/login/:sid`, `POST .../:sid/code` and
//! `POST /api/engines/claude/logout`.

use super::engine_routes::EnginesResponse;
use super::AppState;
use crate::drivers::engines::{EngineId, ProbeEnv};
use crate::profiles::login::{LoginManager, LoginStatus, LoginView, OnSuccess, StartError};
use crate::profiles::{create_private_dir, CliEnv, Engine};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Owner recorded on engine sign-ins, so profile routes never see them (profile ids are `prf_…`).
pub const OWNER: &str = "engine:claude";

/// How long `auth logout` may take.
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ClaudeLogin {
    pub logins: LoginManager,
    /// `None`: `claude` on PATH.
    bin: Option<String>,
    /// `None` (always, outside tests): the CLI's env as turns see it, so `CLAUDE_CONFIG_DIR` on
    /// kwnt's process, else `~/.claude`. Set, sign-in, sign-out and the Claude check use this dir.
    config_dir: Option<PathBuf>,
    /// Successful sign-ins the engine registry has re-checked Claude for.
    rechecked: Mutex<HashSet<String>>,
}

impl Default for ClaudeLogin {
    fn default() -> Self {
        Self::new(None, None, LoginManager::default())
    }
}

impl ClaudeLogin {
    pub fn new(bin: Option<String>, config_dir: Option<PathBuf>, logins: LoginManager) -> Self {
        Self { logins, bin, config_dir, rechecked: Default::default() }
    }

    fn bin(&self) -> Option<String> {
        self.bin.clone().or_else(|| crate::drivers::find_cli_binary("claude"))
    }

    /// The config dir the CLI will write the login to.
    fn home(&self) -> Option<PathBuf> {
        self.config_dir.clone().or_else(|| Engine::Claude.system_home())
    }

    fn cli_env(&self) -> CliEnv {
        match &self.config_dir {
            Some(dir) => CliEnv::for_home(Engine::Claude, dir),
            None => CliEnv::default(),
        }
    }

    /// The engine checks, pointed at the same CLI and config dir as sign-in.
    pub fn probe_env(&self, mut env: ProbeEnv) -> ProbeEnv {
        if let Some(bin) = &self.bin {
            env.claude_bin = Some(bin.clone());
        }
        env.claude_config_dir = self.config_dir.clone();
        env
    }

    fn view(&self, v: &LoginView) -> SessionView {
        let rechecked = self.rechecked.lock().unwrap().contains(&v.flow_id);
        let (state, detail) = match v.status {
            LoginStatus::Starting => ("starting", None),
            LoginStatus::Waiting if v.code_sent => ("verifying", None),
            LoginStatus::Waiting => ("waiting_for_code", None),
            // Done only once GET /api/engines shows the new status
            LoginStatus::Succeeded if !rechecked => ("verifying", None),
            LoginStatus::Succeeded => ("done", None),
            // The CLI's own output is never shown: it can carry the code
            LoginStatus::Failed if v.code_sent => {
                ("failed", Some("Claude didn't accept that code. Start sign-in again.".to_string()))
            }
            LoginStatus::Failed => ("failed", v.error.clone()),
            LoginStatus::Expired => ("expired", Some("Sign-in timed out. Start it again.".to_string())),
            LoginStatus::Cancelled => ("cancelled", None),
        };
        // A finished sign-in's link is dead: don't offer it
        let url = matches!(state, "starting" | "waiting_for_code" | "verifying").then(|| v.url.clone()).flatten();
        SessionView {
            session_id: v.flow_id.clone(),
            state,
            detail,
            url,
            expires_at: v.expires_at,
        }
    }

    fn find(&self, sid: &str) -> Option<LoginView> {
        self.logins.get(sid).filter(|v| v.profile_id == OWNER)
    }
}

/// What the UI gets for a sign-in. `url` is the claude.ai link meant for the user.
#[derive(Debug, Serialize)]
pub struct SessionView {
    pub session_id: String,
    /// `starting`, `waiting_for_code`, `verifying`, `done`, `failed`, `expired` or `cancelled`.
    pub state: &'static str,
    pub detail: Option<String>,
    pub url: Option<String>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

impl SessionView {
    fn settling(&self) -> bool {
        matches!(self.state, "starting" | "verifying")
    }
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

async fn recheck_claude(state: &AppState) {
    let env = super::engine_routes::probe_env(state).await;
    state.engines.recheck_one(EngineId::Claude, &env).await;
}

/// `POST /api/engines/claude/login`: starts Claude Code's sign-in. 409 while one is running.
pub async fn start(State(state): State<AppState>) -> Response {
    let login = &state.claude_login;
    let Some(bin) = login.bin() else {
        return err(StatusCode::BAD_REQUEST, "Claude Code isn't installed");
    };
    let Some(home) = login.home() else {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "couldn't find Claude Code's config dir");
    };
    if login.config_dir.is_some() {
        if let Err(e) = create_private_dir(&home) {
            tracing::warn!(error = %e, "could not create the Claude config dir");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "couldn't create Claude Code's config dir");
        }
    }
    let done_state = state.clone();
    let on_success: OnSuccess = Arc::new(move |view: LoginView| {
        let state = done_state.clone();
        tokio::spawn(async move {
            recheck_claude(&state).await;
            state.claude_login.rechecked.lock().unwrap().insert(view.flow_id);
            tracing::info!("claude engine sign-in finished");
        });
    });
    match login
        .logins
        .start_exclusive(Engine::Claude, &bin, OWNER, &home, &login.cli_env(), on_success)
        .await
    {
        Ok(v) => {
            tracing::info!("claude engine sign-in started");
            Json(login.view(&v)).into_response()
        }
        Err(StartError::Busy(v)) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "a Claude sign-in is already running", "session_id": v.flow_id })),
        )
            .into_response(),
        Err(StartError::Failed(e)) => err(StatusCode::BAD_GATEWAY, e),
    }
}

/// `GET /api/engines/claude/login/:sid`
pub async fn get(State(state): State<AppState>, Path(sid): Path<String>) -> Response {
    match state.claude_login.find(&sid) {
        Some(v) => Json(state.claude_login.view(&v)).into_response(),
        None => err(StatusCode::NOT_FOUND, "no such sign-in"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeBody {
    code: String,
}

/// `POST /api/engines/claude/login/:sid/code`: the code claude.ai showed. Answers once Claude Code
/// has accepted or refused it (or after 20s, still `verifying`).
pub async fn submit_code(
    State(state): State<AppState>,
    Path(sid): Path<String>,
    Json(body): Json<CodeBody>,
) -> Response {
    let login = &state.claude_login;
    if login.find(&sid).is_none() {
        return err(StatusCode::NOT_FOUND, "no such sign-in");
    }
    if let Err(e) = login.logins.submit_code(&sid, &body.code).await {
        return err(StatusCode::BAD_REQUEST, e);
    }
    let mut view = login.view(&login.find(&sid).unwrap());
    for _ in 0..200 {
        if !view.settling() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        view = login.view(&login.find(&sid).unwrap());
    }
    Json(view).into_response()
}

/// `DELETE /api/engines/claude/login/:sid`: stops the sign-in and kills the CLI.
pub async fn cancel(State(state): State<AppState>, Path(sid): Path<String>) -> Response {
    let login = &state.claude_login;
    if login.find(&sid).is_none() {
        return err(StatusCode::NOT_FOUND, "no such sign-in");
    }
    if login.logins.cancel(&sid) {
        // Answer once the process is gone
        for _ in 0..50 {
            if login.find(&sid).is_some_and(|v| !v.status.is_active()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

/// `POST /api/engines/claude/logout`: Claude Code's own `auth logout` on the engine's config dir.
/// Answers with every engine, as `GET /api/engines` does.
pub async fn logout(State(state): State<AppState>) -> Response {
    let login = &state.claude_login;
    let Some(bin) = login.bin() else {
        return err(StatusCode::BAD_REQUEST, "Claude Code isn't installed");
    };
    login.logins.cancel_profile(OWNER);
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args(["auth", "logout"]);
    login.cli_env().apply(&mut cmd);
    cmd.env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let ok = matches!(
        tokio::time::timeout(LOGOUT_TIMEOUT, cmd.status()).await,
        Ok(Ok(s)) if s.success()
    );
    recheck_claude(&state).await;
    if !ok {
        return err(StatusCode::BAD_GATEWAY, "Claude Code couldn't sign out");
    }
    tracing::info!("claude engine signed out");
    Json(EnginesResponse { engines: state.engines.snapshot() }).into_response()
}
