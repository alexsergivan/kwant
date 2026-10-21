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

/// Parks the turn that just hit a usage limit, if it can be retried: not already a retry or a
/// discussion round (`no_park`), and in a chat the retry can find again. Returns the line for
/// the notice.
pub(super) async fn park(
    scope: &TurnScope,
    kwant: &Kwant,
    prompt: &str,
    no_park: bool,
    delegation_id: Option<String>,
    retry_at: Option<DateTime<Utc>>,
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
        })
        .await
        .ok()?;
    let Some(row) = row else {
        return Some("⏳ A retry for this chat is already set for when the limit resets.".into());
    };
    tracing::info!(kwant = %kwant.id, parked = %row.id, retry_at = %at, "turn parked until the usage limit resets");
    scope.state.scheduler_wake.notify_one();
    Some(retry_line(at, Utc::now()))
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

async fn retry(state: &AppState, p: &ParkedTurn) -> bool {
    if let Some(d) = &p.delegation_id {
        return super::delegation::retry_parked(state, d).await;
    }
    let Ok(Some(kwant)) = state.db.get_kwant(&p.kwant_id).await else {
        return false;
    };
    let context = p.channel_id.clone().or_else(|| p.thread_id.clone()).unwrap_or_else(|| kwant.id.clone());
    let job = state.turns.start_job([context, p.id.clone()]);
    let Ok(scope) = scope_for(state, &kwant, p.channel_id.clone(), p.thread_id.clone(), &job, p.depth).await else {
        return false;
    };
    let note = scope.message("system", "kwnt", format!("The usage limit has reset: retrying {}'s turn once.", kwant.name));
    scope.save(&note, None).await;
    let prompt = retry_prompt(&p.prompt);
    tracing::info!(kwant = %kwant.id, parked = %p.id, "retrying a parked turn");
    tokio::spawn(async move {
        let _job = job;
        let mut opts = TurnOpts::default();
        opts.retried = true;
        let _ = try_kwant_turn(&scope, &kwant, prompt, opts).await;
    });
    true
}
