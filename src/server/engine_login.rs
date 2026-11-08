//! In-app sign-in for the CLI engines: `POST /api/engines/:engine/login` runs the engine CLI's
//! own login on the config dir the engine itself uses.
//! - Claude: `auth login --claudeai` prints a claude.ai link; claude.ai shows a code the user
//!   pastes back (`POST .../:sid/code`).
//! - Codex (ChatGPT) and Grok: `login --device-auth` prints a link and a device code; the user
//!   enters the code on the site and the CLI finishes by itself, so kwnt notices on its own.
//!
//! The CLI keeps the login. kwnt never reads, stores or logs the token, the code or the link.
//! Also `GET`/`DELETE /api/engines/:engine/login/:sid` and `POST /api/engines/:engine/logout`.

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
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Owner recorded on Claude's engine sign-ins, so profile routes never see them (profile ids
/// are `prf_…`). Codex and Grok use `engine:codex` and `engine:grok`.
pub const OWNER: &str = "engine:claude";

/// The engines kwnt can sign in to from the app.
pub const ENGINES: [Engine; 3] = [Engine::Claude, Engine::Codex, Engine::Grok];

/// The engine named in the path, if it has an in-app sign-in.
pub fn engine_named(name: &str) -> Option<Engine> {
    ENGINES.into_iter().find(|e| e.as_str() == name)
}

fn engine_id(engine: Engine) -> EngineId {
    match engine {
        Engine::Codex => EngineId::Codex,
        Engine::Grok => EngineId::Grok,
        _ => EngineId::Claude,
    }
}

/// The product name people know the engine by.
fn display(engine: Engine) -> &'static str {
    match engine {
        Engine::Codex => "ChatGPT",
        Engine::Grok => "Grok",
        _ => "Claude",
    }
}

/// The CLI's name in "isn't installed".
fn cli_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Codex => "Codex",
        Engine::Grok => "Grok CLI",
        _ => "Claude Code",
    }
}

/// How long `auth logout` may take.
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct EngineLogin {
    engine: Engine,
    owner: String,
    pub logins: LoginManager,
    /// `None`: the engine's CLI on PATH (or where kwnt installs it).
    bin: Option<String>,
    /// `None` (always, outside tests): the CLI's env as turns see it, so `CLAUDE_CONFIG_DIR`,
    /// `CODEX_HOME` or `GROK_HOME` on kwnt's process, else `~/.claude`, `~/.codex` or `~/.grok`.
    /// Set, sign-in and sign-out use this dir.
    config_dir: Option<PathBuf>,
    /// Successful sign-ins the engine registry has re-checked the engine for: `None` when the
    /// engine is now ready, else what the check said (device sign-ins only).
    rechecked: Mutex<HashMap<String, Option<String>>>,
}

/// The Claude engine's sign-in, as before 0.7.6.
pub type ClaudeLogin = EngineLogin;

impl Default for EngineLogin {
    fn default() -> Self {
        Self::system(Engine::Claude)
    }
}

impl EngineLogin {
    /// Claude's sign-in with this CLI and config dir (tests); see [`Self::for_engine`].
    pub fn new(bin: Option<String>, config_dir: Option<PathBuf>, logins: LoginManager) -> Self {
        Self::for_engine(Engine::Claude, bin, config_dir, logins)
    }

    pub fn for_engine(engine: Engine, bin: Option<String>, config_dir: Option<PathBuf>, logins: LoginManager) -> Self {
        Self {
            engine,
            owner: format!("engine:{}", engine.as_str()),
            logins,
            bin,
            config_dir,
            rechecked: Default::default(),
        }
    }

    /// The engine's own CLI and config dir, as turns use them.
    pub fn system(engine: Engine) -> Self {
        Self::for_engine(engine, None, None, LoginManager::default())
    }

    pub fn engine(&self) -> Engine {
        self.engine
    }

    fn bin(&self) -> Option<String> {
        self.bin.clone().or_else(|| crate::drivers::find_cli_binary(self.engine.as_str()))
    }

    /// The config dir the CLI will write the login to.
    fn home(&self) -> Option<PathBuf> {
        self.config_dir.clone().or_else(|| self.engine.system_home())
    }

    fn cli_env(&self) -> CliEnv {
        match &self.config_dir {
            Some(dir) => CliEnv::for_home(self.engine, dir),
            None => CliEnv::default(),
        }
    }

    fn device(&self) -> bool {
        self.engine != Engine::Claude
    }

    /// The engine checks, pointed at the same CLI (and, for Claude, config dir) as sign-in.
    pub fn probe_env(&self, mut env: ProbeEnv) -> ProbeEnv {
        match self.engine {
            Engine::Codex => {
                if let Some(bin) = &self.bin {
                    env.codex_bin = Some(bin.clone());
                }
            }
            Engine::Grok => {
                if let Some(bin) = &self.bin {
                    env.grok_bin = Some(bin.clone());
                }
            }
            _ => {
                if let Some(bin) = &self.bin {
                    env.claude_bin = Some(bin.clone());
                }
                env.claude_config_dir = self.config_dir.clone();
            }
        }
        env
    }

    fn view(&self, v: &LoginView) -> SessionView {
        let rechecked = self.rechecked.lock().unwrap().get(&v.flow_id).cloned();
        let name = display(self.engine);
        let (state, detail) = match v.status {
            LoginStatus::Starting => ("starting", None),
            // A device sign-in shows nothing until it has both the link and the code
            LoginStatus::Waiting if self.device() && !v.shows_everything() => ("starting", None),
            LoginStatus::Waiting if self.device() => ("waiting_for_approval", None),
            LoginStatus::Waiting if v.code_sent => ("verifying", None),
            LoginStatus::Waiting => ("waiting_for_code", None),
            // Done only once GET /api/engines shows the new status
            LoginStatus::Succeeded => match rechecked {
                None => ("verifying", None),
                Some(None) => ("done", None),
                Some(Some(why)) if self.device() => (
                    "failed",
                    Some(format!("Signed in, but {} can't use this account: {why}", cli_name(self.engine))),
                ),
                Some(Some(_)) => ("done", None),
            },
            // The CLI's own output is never shown: it can carry the code
            LoginStatus::Failed if v.code_sent => {
                ("failed", Some("Claude didn't accept that code. Start sign-in again.".to_string()))
            }
            LoginStatus::Failed => ("failed", v.error.clone()),
            LoginStatus::Expired if self.device() => {
                ("expired", Some(format!("That {name} code expired. Get a new code to try again.")))
            }
            LoginStatus::Expired => ("expired", Some("Sign-in timed out. Start it again.".to_string())),
            LoginStatus::Cancelled => ("cancelled", None),
        };
        // A finished sign-in's link and code are dead: don't offer them
        let live = matches!(state, "starting" | "waiting_for_code" | "waiting_for_approval" | "verifying");
        SessionView {
            session_id: v.flow_id.clone(),
            engine: self.engine.as_str(),
            kind: if self.device() { "device_code" } else { "paste_code" },
            state,
            detail,
            url: live.then(|| v.url.clone()).flatten(),
            user_code: (live && self.device()).then(|| v.user_code.clone()).flatten(),
            expires_at: v.expires_at,
        }
    }

    fn find(&self, sid: &str) -> Option<LoginView> {
        self.logins.get(sid).filter(|v| v.profile_id == self.owner)
    }
}

/// The sign-in of the engine named in the path, or a 404.
#[allow(clippy::result_large_err)]
fn login_for<'a>(state: &'a AppState, name: &str) -> Result<&'a Arc<EngineLogin>, Response> {
    match engine_named(name) {
        Some(Engine::Claude) => Ok(&state.claude_login),
        Some(Engine::Codex) => Ok(&state.codex_login),
        Some(Engine::Grok) => Ok(&state.grok_login),
        _ => Err(err(StatusCode::NOT_FOUND, "no in-app sign-in for this engine")),
    }
}

/// What the UI gets for a sign-in. `url` and `user_code` are meant for the user.
#[derive(Debug, Serialize)]
pub struct SessionView {
    pub session_id: String,
    /// `claude`, `codex` or `grok`.
    pub engine: &'static str,
    /// `paste_code` (Claude: open the link, paste the code back) or `device_code` (Codex, Grok:
    /// open the link, enter `user_code` there; kwnt notices by itself).
    pub kind: &'static str,
    /// `starting`, `waiting_for_code` (paste), `waiting_for_approval` (device), `verifying`,
    /// `done`, `failed`, `expired` or `cancelled`.
    pub state: &'static str,
    pub detail: Option<String>,
    pub url: Option<String>,
    /// The device code to enter on `url` (device sign-ins, while live).
    pub user_code: Option<String>,
    /// When the sign-in (and a device code) stops working.
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

async fn recheck(state: &AppState, engine: Engine) {
    let env = super::engine_routes::probe_env(state).await;
    state.engines.recheck_one(engine_id(engine), &env).await;
}

/// Why the engine can't take turns after a sign-in, or `None` when it is ready.
fn not_ready_reason(state: &AppState, engine: Engine) -> Option<String> {
    let id = engine_id(engine);
    let report = state.engines.snapshot().into_iter().find(|r| r.id == id)?;
    use crate::drivers::engines::EngineStatus;
    // Turned off in Settings is not about the account: the sign-in itself is done
    match report.status {
        EngineStatus::SignedOut => Some("it still shows as signed out".into()),
        EngineStatus::Error => Some(report.detail.unwrap_or_else(|| "it isn't ready".into())),
        _ => None,
    }
}

/// `POST /api/engines/:engine/login`: starts the engine CLI's sign-in. 409 while one is running.
pub async fn start(State(state): State<AppState>, Path(engine): Path<String>) -> Response {
    let login = match login_for(&state, &engine) {
        Ok(l) => l.clone(),
        Err(r) => return r,
    };
    let engine = login.engine;
    let Some(bin) = login.bin() else {
        return err(StatusCode::BAD_REQUEST, format!("{} isn't installed", cli_name(engine)));
    };
    let Some(home) = login.home() else {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("couldn't find {}'s config dir", cli_name(engine)));
    };
    // Codex refuses a CODEX_HOME that doesn't exist yet
    if login.config_dir.is_some() || !home.exists() {
        if let Err(e) = create_private_dir(&home) {
            tracing::warn!(engine = engine.as_str(), error = %e, "could not create the engine's config dir");
            return err(StatusCode::INTERNAL_SERVER_ERROR, format!("couldn't create {}'s config dir", cli_name(engine)));
        }
    }
    let done_state = state.clone();
    let on_success: OnSuccess = Arc::new(move |view: LoginView| {
        let state = done_state.clone();
        tokio::spawn(async move {
            recheck(&state, engine).await;
            let login = login_for(&state, engine.as_str()).expect("engine has a sign-in");
            let why = if login.device() { not_ready_reason(&state, engine) } else { None };
            let ready = why.is_none();
            login.rechecked.lock().unwrap().insert(view.flow_id, why);
            tracing::info!(engine = engine.as_str(), ready, "{} engine sign-in finished", engine.as_str());
        });
    });
    match login
        .logins
        .start_exclusive(engine, &bin, &login.owner, &home, &login.cli_env(), on_success)
        .await
    {
        Ok(v) => {
            tracing::info!(engine = engine.as_str(), "{} engine sign-in started", engine.as_str());
            Json(login.view(&v)).into_response()
        }
        Err(StartError::Busy(v)) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("a {} sign-in is already running", display(engine)),
                "session_id": v.flow_id,
            })),
        )
            .into_response(),
        Err(StartError::Failed(e)) => err(StatusCode::BAD_GATEWAY, e),
    }
}

/// `GET /api/engines/:engine/login/:sid`
pub async fn get(State(state): State<AppState>, Path((engine, sid)): Path<(String, String)>) -> Response {
    let login = match login_for(&state, &engine) {
        Ok(l) => l,
        Err(r) => return r,
    };
    match login.find(&sid) {
        Some(v) => Json(login.view(&v)).into_response(),
        None => err(StatusCode::NOT_FOUND, "no such sign-in"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeBody {
    code: String,
}

/// `POST /api/engines/:engine/login/:sid/code`: the code claude.ai showed (Claude only). Answers
/// once Claude Code has accepted or refused it (or after 20s, still `verifying`).
pub async fn submit_code(
    State(state): State<AppState>,
    Path((engine, sid)): Path<(String, String)>,
    Json(body): Json<CodeBody>,
) -> Response {
    let login = match login_for(&state, &engine) {
        Ok(l) => l,
        Err(r) => return r,
    };
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

/// `DELETE /api/engines/:engine/login/:sid`: stops the sign-in and kills the CLI.
pub async fn cancel(State(state): State<AppState>, Path((engine, sid)): Path<(String, String)>) -> Response {
    let login = match login_for(&state, &engine) {
        Ok(l) => l,
        Err(r) => return r,
    };
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

/// `POST /api/engines/:engine/logout`: the CLI's own sign-out (`claude auth logout`,
/// `codex logout`, `grok logout`) on the engine's config dir. Answers with every engine, as
/// `GET /api/engines` does.
pub async fn logout(State(state): State<AppState>, Path(engine): Path<String>) -> Response {
    let login = match login_for(&state, &engine) {
        Ok(l) => l.clone(),
        Err(r) => return r,
    };
    let engine = login.engine;
    let Some(bin) = login.bin() else {
        return err(StatusCode::BAD_REQUEST, format!("{} isn't installed", cli_name(engine)));
    };
    login.logins.cancel_profile(&login.owner);
    let mut cmd = tokio::process::Command::new(&bin);
    match engine {
        Engine::Claude => cmd.args(["auth", "logout"]),
        _ => cmd.arg("logout"),
    };
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
    recheck(&state, engine).await;
    if !ok {
        return err(StatusCode::BAD_GATEWAY, format!("{} couldn't sign out", cli_name(engine)));
    }
    tracing::info!(engine = engine.as_str(), "{} engine signed out", engine.as_str());
    Json(EnginesResponse { engines: state.engines.snapshot() }).into_response()
}
