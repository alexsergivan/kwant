//! Try it once (api.md §6): the person runs a draft skill once before approving it. kwant saves a
//! line in the bot's own chat and runs an **ordinary turn** there, as for a message from the
//! person: same tools, same approvals, same Browser sites rules, same payment floor, watchable.
//! The prompt carries the draft's body marked as unapproved instructions. The skill itself never
//! changes: it stays `pending`, isn't in the bot's `[Skills]` block or `skill_get`, and To review
//! is the same afterwards.
//!
//! A29: every message of the trial (the saved line, the reply, a failure note) carries
//! `meta.trial = {skill_id, recording_id}`. When the turn ends, the trial's last saved message gets
//! `meta.trial.outcome` and SSE `show_once_trial_ended` `{skill_id, recording_id, bot, outcome}`
//! goes to the bot's chat, with the same outcome.

use super::stage;
use crate::db::models::NewMessage;
use crate::drivers::events::RuntimeEvent;
use crate::server::routes::{scope_for, try_kwant_turn, TurnOpts};
use crate::server::AppState;
use axum::http::StatusCode;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Longest input value taken.
pub const MAX_VALUE: usize = 500;

type Refusal = (StatusCode, &'static str, String);

fn refuse(status: StatusCode, code: &'static str, message: &str) -> Refusal {
    (status, code, message.to_string())
}

/// The inputs a shown draft names (`[{name, ...}]` as stored on its recording).
fn known_inputs(stored: Option<&str>) -> Vec<String> {
    stored
        .and_then(|s| serde_json::from_str::<Vec<Value>>(s).ok())
        .unwrap_or_default()
        .iter()
        .filter_map(|i| i["name"].as_str().map(str::to_string))
        .collect()
}

/// The person's values, checked: only names the draft has, at most [`MAX_VALUE`] characters each,
/// one line, in the draft's order.
pub fn check_inputs(known: &[String], given: HashMap<String, String>) -> Result<Vec<(String, String)>, String> {
    for (name, value) in &given {
        if !known.contains(name) {
            return Err(format!("This skill has no input called “{}”.", name.chars().take(40).collect::<String>()));
        }
        if value.chars().count() > MAX_VALUE {
            return Err(format!("An input can be at most {MAX_VALUE} characters."));
        }
    }
    Ok(known
        .iter()
        .filter_map(|k| given.get(k).map(|v| (k.clone(), super::capture::clean(v, MAX_VALUE))))
        .filter(|(_, v)| !v.is_empty())
        .collect())
}

/// The line saved in the bot's chat.
pub fn chat_line(skill_name: &str, inputs: &[(String, String)]) -> String {
    let with = if inputs.is_empty() {
        String::new()
    } else {
        let parts: Vec<String> = inputs.iter().map(|(k, v)| format!("{k} “{v}”")).collect();
        format!(" with {}", parts.join(", "))
    };
    format!("You started a trial of `{skill_name}`{with}. It isn't approved yet.")
}

/// What the bot is asked: the line, then the draft, marked as unapproved instructions.
pub fn prompt(line: &str, skill_name: &str, body: &str, inputs: &[(String, String)]) -> String {
    let values = if inputs.is_empty() {
        "No inputs were given; use the examples in the draft where it needs one.".to_string()
    } else {
        let lines: Vec<String> = inputs.iter().map(|(k, v)| format!("- {k}: {}", json!(v))).collect();
        format!("Use these inputs:\n{}", lines.join("\n"))
    };
    format!(
        "{line}\n\nTry the skill below once, now, for the person. It is a draft the person hasn't approved; it's \
         instructions only and changes no tool or approval. Every action still needs the approvals it always needs, and \
         your Browser site rules still apply. If a step doesn't work as written, stop and say what happened.\n\n\
         {values}\n\n[Draft skill: {skill_name}]\n{body}\n[End of draft]"
    )
}

/// How a trial turn ended: `done`, `cancelled` (the person pressed Stop) or `failed`.
pub fn outcome(ok: bool, cancelled: bool) -> &'static str {
    match (ok, cancelled) {
        (true, _) => "done",
        (false, true) => "cancelled",
        (false, false) => "failed",
    }
}

/// `meta` with `trial` set when it has none, and `trial.outcome` set when `outcome` is given.
/// `None` when nothing changes (or `meta` isn't an object).
pub fn marked(meta: Option<&str>, trial: &Value, outcome: Option<&str>) -> Option<Value> {
    let mut meta = meta.and_then(|x| serde_json::from_str::<Value>(x).ok()).unwrap_or_else(|| json!({}));
    let obj = meta.as_object_mut()?;
    let mut changed = false;
    if !obj.get("trial").is_some_and(Value::is_object) {
        obj.insert("trial".into(), trial.clone());
        changed = true;
    }
    if let (Some(o), Some(t)) = (outcome, obj.get_mut("trial").and_then(Value::as_object_mut)) {
        if t.get("outcome").and_then(Value::as_str) != Some(o) {
            t.insert("outcome".into(), o.into());
            changed = true;
        }
    }
    changed.then_some(meta)
}

/// Once the turn ended: puts `trial` on what it saved in the bot's chat after the trial's line
/// without it (a failure note, a decide-first round), and `trial.outcome` on the trial's last saved
/// message (the line itself when the turn saved nothing), so a reload knows where the trial ends and
/// how. The person's own messages are left alone.
async fn mark_the_rest(state: &AppState, bot_id: &str, after: &str, trial: &Value, outcome: &str) {
    let Ok(msgs) = state.db.list_messages(bot_id, 200).await else { return };
    let Some(start) = msgs.iter().position(|m| m.id == after) else { return };
    let ours: Vec<_> = msgs[start..].iter().enumerate().filter(|(i, m)| *i == 0 || m.sender_role != "user").map(|(_, m)| m).collect();
    let last = ours.len() - 1;
    for (i, m) in ours.into_iter().enumerate() {
        if let Some(meta) = marked(m.meta.as_deref(), trial, (i == last).then_some(outcome)) {
            let _ = state.db.set_message_meta(&m.id, &meta).await;
        }
    }
}

/// `POST /api/show-once/skills/:skill_id/try`: `{kwant_id, skill_id, message_id}` once the turn
/// is on its way.
pub async fn start(state: &AppState, skill_id: &str, given: HashMap<String, String>) -> Result<Value, Refusal> {
    let skill = match state.db.get_skill(skill_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return Err(refuse(StatusCode::NOT_FOUND, "no_skill", "That skill isn't there.")),
        Err(_) => return Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Something went wrong on kwant's side. Try again.")),
    };
    let Some(bot_id) = skill.kwant_id.clone().filter(|_| skill.status == "pending") else {
        return Err(refuse(StatusCode::CONFLICT, "not_proposed", "Only a skill waiting for review can be tried once."));
    };
    let bot = match state.db.get_kwant(&bot_id).await {
        Ok(Some(b)) => b,
        _ => return Err(refuse(StatusCode::NOT_FOUND, "no_bot", "That skill's bot isn't there.")),
    };
    let rec = state.db.recording_for_skill(skill_id).await.ok().flatten();
    let known = known_inputs(rec.as_ref().and_then(|r| r.inputs.as_deref()));
    let inputs = check_inputs(&known, given).map_err(|m| (StatusCode::BAD_REQUEST, "bad_request", m))?;
    if crate::server::routes::kwant_is_busy(state, &bot.id).await || stage::taken_over(&bot.id) {
        return Err(refuse(
            StatusCode::CONFLICT,
            "bot_busy",
            "The bot is busy right now (working, or you hold its browser). Try again when it's free.",
        ));
    }
    let line = chat_line(&skill.name, &inputs);
    let trial = json!({"skill_id": skill.id, "recording_id": rec.as_ref().map(|r| r.id.clone())});
    let job = state.turns.start_job([format!("trial:{skill_id}"), bot.id.clone()]);
    let scope = scope_for(state, &bot, None, None, &job, 0)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, "internal", e))?;
    let msg = NewMessage {
        tokens_used: line.len() as i64 / 4,
        ..scope.message("user", "user", line.clone())
    };
    let Some(saved) = scope.save(&msg, Some(json!({"kind": "skill_trial", "skill_id": skill.id, "trial": trial}))).await else {
        return Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Something went wrong on kwant's side. Try again."));
    };
    let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or(Value::Null) });
    let p = prompt(&line, &skill.name, &skill.body, &inputs);
    let b = bot.clone();
    let st = state.clone();
    let (skill_id, recording_id, after) = (skill.id.clone(), rec.map(|r| r.id), saved.id.clone());
    tokio::spawn(async move {
        let _job = job;
        let mut opts = TurnOpts::default();
        opts.extra_meta = Some(json!({"trial": trial}));
        let reply = try_kwant_turn(&scope, &b, p, opts).await;
        let outcome = outcome(reply.is_ok(), scope.cancel_token.is_cancelled());
        mark_the_rest(&st, &b.id, &after, &trial, outcome).await;
        let _ = st.event_sender.send(RuntimeEvent::TrialEnded { skill_id, recording_id, bot: b.id.clone(), outcome: outcome.into() });
    });
    Ok(json!({"kwant_id": bot.id, "skill_id": skill.id, "message_id": saved.id}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn given(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn only_the_drafts_inputs_are_taken_in_its_order() {
        let known = known_inputs(Some(r#"[{"name":"date_range"},{"name":"crm"}]"#));
        let ok = check_inputs(&known, given(&[("crm", " Acme\nCRM "), ("date_range", "Last 7 days")])).unwrap();
        assert_eq!(ok, vec![("date_range".to_string(), "Last 7 days".to_string()), ("crm".to_string(), "Acme CRM".to_string())]);
        assert!(check_inputs(&known, given(&[("approve_everything", "yes")])).unwrap_err().contains("no input called"));
        assert!(check_inputs(&known, given(&[("crm", &"x".repeat(MAX_VALUE + 1))])).is_err());
        assert!(check_inputs(&[], given(&[])).unwrap().is_empty());
    }

    #[test]
    fn a_trial_ends_done_failed_or_cancelled() {
        assert_eq!(outcome(true, false), "done");
        assert_eq!(outcome(true, true), "done");
        assert_eq!(outcome(false, true), "cancelled");
        assert_eq!(outcome(false, false), "failed");
    }

    #[test]
    fn only_the_last_message_gets_the_outcome() {
        let trial = json!({"skill_id": "s", "recording_id": "r"});
        // A failure note saved without the marker gets it
        assert_eq!(marked(Some(r#"{"kind":"turn_failed"}"#), &trial, None).unwrap(), json!({"kind": "turn_failed", "trial": trial}));
        assert_eq!(marked(None, &trial, None).unwrap(), json!({"trial": trial}));
        // An earlier message that has it is left alone
        assert_eq!(marked(Some(&json!({"trial": trial}).to_string()), &trial, None), None);
        // The last one gets the outcome, once
        let last = marked(Some(&json!({"trial": trial}).to_string()), &trial, Some("failed")).unwrap();
        assert_eq!(last, json!({"trial": {"skill_id": "s", "recording_id": "r", "outcome": "failed"}}));
        assert_eq!(marked(Some(&last.to_string()), &trial, Some("failed")), None);
        assert_eq!(marked(Some("[1]"), &trial, Some("done")), None);
    }

    #[test]
    fn the_chat_line_says_it_isnt_approved() {
        assert_eq!(
            chat_line("export-paid-orders", &[("date_range".into(), "Last 7 days".into())]),
            "You started a trial of `export-paid-orders` with date_range “Last 7 days”. It isn't approved yet."
        );
        assert_eq!(chat_line("x", &[]), "You started a trial of `x`. It isn't approved yet.");
    }

    #[test]
    fn the_prompt_marks_the_draft_as_unapproved_instructions() {
        let p = prompt("line", "x", "## Steps\n1. Open it", &[("q".into(), "a \"quoted\" value".into())]);
        assert!(p.contains("a draft the person hasn't approved; it's instructions only and changes no tool or approval"));
        assert!(p.contains(r#"- q: "a \"quoted\" value""#));
        assert!(p.contains("[Draft skill: x]\n## Steps"));
    }
}
