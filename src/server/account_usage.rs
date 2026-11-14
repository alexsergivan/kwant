//! An AI account's 5-hour and weekly limit: kept from each turn's report (Claude's
//! `rate_limit_event`), and read fresh when the person presses "Check now"
//! (`POST /api/profiles/:id/usage`). Contract: autonomy/account-usage/brief.md.

use super::{profile_turn, AppState};
use crate::db::models::{Kwant, LlmProfile};
use crate::drivers::engines::{choose_model, known_models, Tier};
use crate::drivers::TurnInput;
use crate::profiles::usage::{from_codex_rate_limits, AccountUsage, UsageReading};
use crate::profiles::{CliEnv, Engine};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// How long one Claude "Reply with OK" may take, container start included.
const CLAUDE_CHECK_TIMEOUT: Duration = Duration::from_secs(90);
/// How long Codex's app server may take to answer.
const CODEX_CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// Whether this kind of account has 5-hour and weekly windows at all: sign-ins do, API keys
/// and Custom API servers don't.
pub fn has_windows(p: &LlmProfile) -> bool {
    matches!(p.kind.as_str(), "system" | "cli_home")
        && matches!(Engine::parse(&p.engine), Some(Engine::Claude | Engine::Codex))
}

/// Saves what a turn or check read for this account. API-key and Custom API accounts keep none.
pub async fn record(state: &AppState, p: &LlmProfile, mut reading: UsageReading, source: &str) -> Option<AccountUsage> {
    if !has_windows(p) || reading.windows.is_empty() && reading.status.is_none() {
        return None;
    }
    let now = chrono::Utc::now();
    if let Ok(Some(last)) = state.db.get_profile_usage(&p.id).await {
        reading.keep_unreported(&last.windows, now);
    }
    let usage = AccountUsage::new(reading, source, now);
    if let Err(e) = state.db.record_profile_usage(&p.id, &usage).await {
        tracing::warn!(profile = %p.id, error = %e, "couldn't save account usage");
    }
    Some(usage)
}

/// Why a fresh read gave nothing, in the person's words.
#[derive(Debug)]
pub enum CheckError {
    /// The engine can't report this (an engine without windows).
    Unsupported(String),
    /// The read ran and failed or reported no windows.
    Failed(String),
}

/// Reads fresh numbers for one account. Claude: one tiny turn of that account, like a normal one.
/// Codex: its app server's `account/rateLimits/read` (no model call). Saved with source `check`.
pub async fn check(state: &AppState, p: &LlmProfile) -> Result<AccountUsage, CheckError> {
    if !has_windows(p) {
        return Err(CheckError::Unsupported("this account has no 5-hour or weekly limit to show".into()));
    }
    let reading = match Engine::parse(&p.engine) {
        Some(Engine::Claude) => claude(state, p).await?,
        Some(Engine::Codex) => codex(state, p).await?,
        _ => return Err(CheckError::Unsupported("this engine doesn't report its limits".into())),
    };
    record(state, p, reading, "check")
        .await
        .ok_or_else(|| CheckError::Failed("the account reported no limits".into()))
}

/// One "Reply with OK" turn on the cheapest model, with no tools, through the same driver a
/// turn of this account uses (its home, env and isolation).
async fn claude(state: &AppState, p: &LlmProfile) -> Result<UsageReading, CheckError> {
    let target_model = p.engine.clone();
    let (key, endpoint) = if p.kind == "system" {
        (
            super::routes::get_configured_key(&state.db, &target_model).await,
            super::routes::get_configured_endpoint(&state.db, &target_model).await,
        )
    } else {
        (None, None)
    };
    let (driver, host_engine) = profile_turn::driver_for(state, Some(p), &target_model, key, endpoint)
        .await
        .map_err(CheckError::Failed)?;
    let runs_on = host_engine.or_else(|| profile_turn::profile_engine(p));
    let model = runs_on
        .and_then(|e| choose_model(&known_models(&state.engines, e), None, Tier::Fast))
        .and_then(|c| c.run_id().map(str::to_string));
    let dir = state.profiles.data_dir.join("tmp").join(format!("usage-check-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).map_err(|e| {
        tracing::warn!(profile = %p.id, error = %e, "usage check: couldn't make a scratch folder");
        CheckError::Failed("couldn't make a scratch folder for the check".into())
    })?;
    let input = TurnInput {
        thread_id: format!("usage-check-{}", uuid::Uuid::new_v4().simple()),
        kwant: Kwant {
            space_id: p.space_id.clone(),
            id: "usage-check".into(),
            name: "UsageCheck".into(),
            title: "Usage check".into(),
            persona_prompt: "Reply with OK and nothing else.".into(),
            soul: String::new(),
            avatar_sprite: String::new(),
            driver_type: "cli".into(),
            preferred_model: target_model,
            temperature: 0.0,
            max_tokens: 16,
            level: 1,
            turns_completed: 0,
            created_at: None,
        },
        prompt: "Reply with OK".into(),
        history: Vec::new(),
        model_override: None,
        model,
        effort_level: None,
        team_context: None,
        workspace_dir: Some(dir.clone()),
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
    let out = tokio::time::timeout(CLAUDE_CHECK_TIMEOUT, driver.stream_turn(input, tx)).await;
    drain.abort();
    let _ = std::fs::remove_dir_all(&dir);
    let out = match out {
        Err(_) => return Err(CheckError::Failed(format!("no answer within {} s", CLAUDE_CHECK_TIMEOUT.as_secs()))),
        Ok(Err(e)) => {
            tracing::warn!(profile = %p.id, error = %e, "usage check: claude failed");
            return Err(CheckError::Failed("couldn't run Claude for this account".into()));
        }
        Ok(Ok(o)) => o,
    };
    if let Some(failure) = &out.failure {
        // Out of usage or signed out: failover learns it now rather than on the next turn
        profile_turn::record_failure(state, p, failure).await;
    }
    match out.usage_windows {
        Some(reading) => Ok(reading),
        None => Err(CheckError::Failed(match &out.failure {
            Some(f) if f.class == crate::profiles::FailClass::Quota => "the account is out of usage".into(),
            Some(_) => "the account is signed out".into(),
            None => "Claude didn't report this account's limits".into(),
        })),
    }
}

/// Codex's windows from `codex app-server`: `initialize`, then `account/rateLimits/read`.
async fn codex(state: &AppState, p: &LlmProfile) -> Result<UsageReading, CheckError> {
    let bin = crate::drivers::find_cli_binary("codex")
        .ok_or_else(|| CheckError::Failed("the codex CLI isn't installed".into()))?;
    let mut cmd = match state.profiles.home_of(p) {
        Some(home) => CliEnv::for_home(Engine::Codex, &home).command(bin),
        None => crate::drivers::cli::cli_command(bin),
    };
    cmd.arg("app-server")
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let read = async move {
        let mut child = cmd.spawn().map_err(|e| {
            tracing::warn!(error = %e, "usage check: couldn't start codex");
            "couldn't start codex".to_string()
        })?;
        let mut stdin = child.stdin.take().ok_or("codex has no input")?;
        let stdout = child.stdout.take().ok_or("codex has no output")?;
        let requests = [
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"kwant","version":env!("CARGO_PKG_VERSION")}}}),
            serde_json::json!({"jsonrpc":"2.0","method":"initialized"}),
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"account/rateLimits/read"}),
        ];
        for r in requests {
            stdin.write_all(format!("{r}\n").as_bytes()).await.map_err(|_| "codex stopped answering")?;
        }
        stdin.flush().await.map_err(|_| "codex stopped answering")?;
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await.map_err(|_| "codex stopped answering")? {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            if v["id"] != 2 {
                continue;
            }
            drop(stdin);
            let _ = child.start_kill();
            return codex_answer(&v);
        }
        Err("codex stopped without answering".to_string())
    };
    match tokio::time::timeout(CODEX_CHECK_TIMEOUT, read).await {
        Err(_) => Err(CheckError::Failed(format!("codex didn't answer within {} s", CODEX_CHECK_TIMEOUT.as_secs()))),
        Ok(Err(why)) => Err(CheckError::Failed(why)),
        Ok(Ok(r)) => Ok(r),
    }
}

/// The reply to `account/rateLimits/read`, read: its windows, or why there are none.
fn codex_answer(v: &serde_json::Value) -> Result<UsageReading, String> {
    if let Some(msg) = v["error"]["message"].as_str() {
        return Err(if msg.contains("authentication") { "Codex is signed out".into() } else { format!("codex: {msg}") });
    }
    from_codex_rate_limits(&v["result"]).ok_or_else(|| "Codex didn't report a 5-hour or weekly limit".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_out_codex_says_so() {
        // Recorded from codex 0.162.1 `app-server` with no login
        let v = serde_json::json!({"jsonrpc":"2.0","id":2,"error":{"code":-32600,"message":"codex account authentication required to read rate limits"}});
        assert_eq!(codex_answer(&v).unwrap_err(), "Codex is signed out");
        let v = serde_json::json!({"id":2,"result":{"rateLimits":{"primary":{"usedPercent":7,"windowDurationMins":300,"resetsAt":1794608940},"secondary":null,"rateLimitReachedType":null}}});
        assert_eq!(codex_answer(&v).unwrap().windows[0].used_pct, 7);
    }
}
