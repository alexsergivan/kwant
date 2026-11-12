//! Turns a usage limit stopped before the bot did anything visible are parked until the limit
//! resets, then run once more (the retry never parks again). The scheduler runs due ones; they're
//! kept in the database, so a restart in between doesn't lose them.

use super::routes::{scope_for, try_kwant_turn, TurnOpts, TurnScope};
use super::AppState;
use crate::db::models::Kwant;
use crate::db::parked::{NewParkedTurn, ParkedTurn};
use chrono::{DateTime, Utc};

/// The line added to the give-up notice when the turn was parked.
pub fn retry_line(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let mins = (at - now).num_minutes().max(1);
    let wait = if mins >= 90 { format!("about {} h", (mins + 30) / 60) } else { format!("about {mins} min") };
    format!("⏳ kwnt will retry this once at {} UTC (in {wait}), when the limit resets.", at.format("%H:%M"))
}

/// Parks the turn that just hit a usage limit, if it can be retried: not already a retry, a
/// discussion round or a no-tools turn (`no_park`, see `TurnOpts::no_park`), and in a chat the
/// retry can find again. Returns the line for
/// the notice.
pub(super) async fn park(
    scope: &TurnScope,
    kwant: &Kwant,
    prompt: &str,
    no_park: bool,
    delegation_id: Option<String>,
    retry_at: Option<DateTime<Utc>>,
) -> Option<String> {
    park_for(scope, kwant, prompt, no_park, delegation_id, retry_at, "usage_limit").await
}

/// [`park`], with why it waits: "usage_limit" or "bot_budget" (G2, see
/// [`super::bot_budget`]). Budget holds keep every message in the chat (appended, oldest first).
pub(super) async fn park_for(
    scope: &TurnScope,
    kwant: &Kwant,
    prompt: &str,
    no_park: bool,
    delegation_id: Option<String>,
    retry_at: Option<DateTime<Utc>>,
    reason: &'static str,
) -> Option<String> {
    if no_park {
        return None;
    }
    let at = retry_at?;
    // A direct chat with another bot (a hand-off) isn't a chat this bot's retry can open
    if scope.kwant_id.as_deref().is_some_and(|k| k != kwant.id) {
        return None;
    }
    let row = scope
        .state
        .db
        .park_turn(&NewParkedTurn {
            kwant_id: kwant.id.clone(),
            channel_id: scope.channel_id.clone(),
            direct_kwant_id: scope.kwant_id.clone(),
            thread_id: scope.thread_id.clone(),
            delegation_id,
            depth: scope.delegation_depth,
            prompt: prompt.to_string(),
            retry_at: at,
            reason,
        })
        .await
        .ok()?;
    let budget = reason == "bot_budget";
    let Some(row) = row else {
        return Some(if budget {
            "⏳ Held with the earlier message: it runs when the bot can start new work again.".into()
        } else {
            "⏳ A retry for this chat is already set for when the limit resets.".into()
        });
    };
    if budget {
        tracing::info!(kwant = %kwant.id, parked = %row.id, "turn held: the bot is paused at its monthly budget");
        scope.state.scheduler_wake.notify_one();
        return Some(format!("⏳ Held: it runs when {} can start new work again.", kwant.name));
    }
    tracing::info!(kwant = %kwant.id, parked = %row.id, retry_at = %at, "turn parked until the usage limit resets");
    scope.state.scheduler_wake.notify_one();
    Some(retry_line(at, Utc::now()))
}

/// The reason a reply a restart cut off waits to run again (see [`super::restart_resume`]); its
/// parked prompt already says what was cut off.
pub const RESTART: &str = "restart";

/// The prompt a held turn gets once its bot can start new work again (G2).
pub fn budget_retry_prompt(original: &str) -> String {
    format!(
        "[kwnt held this while you were paused at your monthly budget; the person has raised or continued it, or a new month started. Answer it now]\n\n{original}"
    )
}

/// The prompt a retried turn gets: the original request, marked as a retry.
pub fn retry_prompt(original: &str) -> String {
    format!(
        "[kwnt is retrying your last turn: a usage limit stopped it before you could reply, and the limit has reset]\n\n{original}"
    )
}

/// Runs the parked turns due by `now`, each once. Returns how many it started.
pub async fn run_due(state: &AppState, now: DateTime<Utc>) -> usize {
    let Ok(due) = state.db.take_due_parked(now).await else {
        return 0;
    };
    let mut started = 0;
    for p in due {
        if retry(state, &p).await {
            started += 1;
        }
        let _ = state.db.set_parked_status(&p.id, "done").await;
    }
    started
}

/// Test hook: one no-tools turn (`TurnOpts::no_tools`, as a Dev mode card or an idea take runs)
/// for `kwant_id` in its direct chat. Returns the reply, or the failure text.
#[doc(hidden)]
pub async fn no_tools_turn_for_tests(state: &AppState, kwant_id: &str, prompt: &str) -> Result<String, String> {
    let kwant = state.db.get_kwant(kwant_id).await.ok().flatten().ok_or("no such bot")?;
    let job = state.turns.start_job([kwant.id.clone()]);
    let scope = scope_for(state, &kwant, None, None, &job, 0).await?;
    let mut opts = TurnOpts::default();
    opts.no_tools = true;
    try_kwant_turn(&scope, &kwant, prompt.to_string(), opts).await
}

async fn retry(state: &AppState, p: &ParkedTurn) -> bool {
    if let Some(d) = &p.delegation_id {
        return super::delegation::retry_parked_for(state, d, p.reason == "bot_budget").await;
    }
    let Ok(Some(kwant)) = state.db.get_kwant(&p.kwant_id).await else {
        return false;
    };
    let context = p.channel_id.clone().or_else(|| p.thread_id.clone()).unwrap_or_else(|| kwant.id.clone());
    let job = state.turns.start_job([context, p.id.clone()]);
    let Ok(scope) = scope_for(state, &kwant, p.channel_id.clone(), p.thread_id.clone(), &job, p.depth).await else {
        return false;
    };
    let budget = p.reason == "bot_budget";
    let restart = p.reason == RESTART;
    // 0.9.5: a busy-browser retry (see [`super::busy_retry`]); its count rides on the reason
    let busy = super::busy_retry::retries_of(&p.reason);
    // A restart's continuation and a busy-browser retry said so in the chat when they were parked
    if !restart && busy.is_none() {
        let text = if budget {
            format!("{} can start new work again: running the message it held.", kwant.name)
        } else {
            format!("The usage limit has reset: retrying {}'s turn once.", kwant.name)
        };
        let note = scope.message("system", "kwnt", text);
        scope.save(&note, None).await;
    }
    let prompt = if budget {
        budget_retry_prompt(&p.prompt)
    } else if restart || busy.is_some() {
        p.prompt.clone()
    } else {
        retry_prompt(&p.prompt)
    };
    tracing::info!(kwant = %kwant.id, parked = %p.id, reason = %p.reason, "retrying a parked turn");
    tokio::spawn(async move {
        let _job = job;
        let mut opts = TurnOpts::default();
        opts.retried = true;
        opts.resumed_after_restart = restart;
        opts.busy_retries = busy.unwrap_or(0);
        let _ = try_kwant_turn(&scope, &kwant, prompt, opts).await;
    });
    true
}
