//! The bridge between one Telegram chat and a space: what comes in (G3 pairing, G4 messages,
//! button taps), what goes out (A19 replies, G5 needs-you alerts, G6 approvals) and the timers
//! (pairing expiry, the reply window).
//!
//! Everything read from Telegram is the person's text or a stranger's, never an instruction to
//! kwant: it goes to #general as a chat message (where it is untrusted chat text like any other)
//! or is dropped. Nothing a bot writes can pair an account, allow an approval or switch anything.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{api, copy, db, emit_status, pairing, runner, text, AppState, Out, Runner, Status};
use crate::broker::ApprovalDecision;
use crate::db::spaces::SpaceId;
use crate::db::telegram::{TelegramAsk, TelegramLink};
use crate::drivers::events::RuntimeEvent;

/// How long bot replies in #general keep going back after the person last wrote from Telegram,
/// unless they write in kwant first (A19).
pub const REPLY_WINDOW_HOURS: i64 = 6;
/// How often the timers run (pairing expiry, the reply sweep, answered approvals).
const TICK: Duration = Duration::from_secs(2);
/// Notify kinds that mean a bot is stuck or stopped (G5 "stuck").
const STUCK_KINDS: &[&str] = &["delegation_stalled", "routine_failed", "spend_stopped"];
/// Notify kinds that mean work is done (G5 "done", off by default).
const DONE_KINDS: &[&str] = &["turn_finished", "delegation_finished"];
/// Choice buttons per row (design G5: two per row).
const PER_ROW: usize = 2;
/// The "✓" line added to an answered message.
const TEAL_CHECK: &str = "✓";

// --- Inbound -------------------------------------------------------------------------------

/// Long-polls `getUpdates` until the runner stops, from the stored offset, so a restart resumes
/// where it left off (G8) and an update stored before a crash is never posted twice.
pub async fn poll_loop(state: AppState, runner: Arc<Runner>) {
    let mut backoff = 1u64;
    loop {
        if runner.cancel.is_cancelled() {
            return;
        }
        let Ok(Some(link)) = state.db.telegram_link(&runner.space).await else {
            super::stop(&state, &runner.space);
            return;
        };
        if link.state == db::REJECTED {
            return;
        }
        let got = tokio::select! {
            _ = runner.cancel.cancelled() => return,
            r = runner.client.get_updates(link.update_offset, api::POLL_SECS) => r,
        };
        let pause = match got {
            Ok(updates) => {
                backoff = 1;
                if runner.set_status(Status::Listening) {
                    emit_status(&state, &runner.space);
                }
                if link.last_error.is_some() {
                    let _ = state.db.telegram_set_error(&runner.space, None).await;
                }
                for u in updates {
                    let Some(id) = u["update_id"].as_i64() else { continue };
                    if state.db.telegram_claim_update(&runner.space, id).await.unwrap_or(false) {
                        handle_update(&state, &runner, &u).await;
                    }
                    let _ = state.db.telegram_set_offset(&runner.space, id + 1).await;
                }
                None
            }
            Err(api::TgError::Unauthorized) => {
                rejected(&state, &runner).await;
                return;
            }
            Err(api::TgError::TooMany { retry_after }) => {
                if runner.set_status(Status::Throttled) {
                    emit_status(&state, &runner.space);
                }
                Some(Duration::from_secs(retry_after.clamp(1, 60)))
            }
            Err(e) => {
                // Offline, Telegram down, or another program polling with the same token (409)
                if runner.set_status(Status::Offline) {
                    emit_status(&state, &runner.space);
                }
                if matches!(e, api::TgError::Conflict) {
                    let _ = state.db.telegram_set_error(&runner.space, Some("conflict")).await;
                }
                let wait = backoff;
                backoff = (backoff * 2).min(30);
                Some(Duration::from_secs(wait))
            }
        };
        if let Some(wait) = pause {
            tokio::select! {
                _ = runner.cancel.cancelled() => return,
                _ = tokio::time::sleep(wait) => {}
            }
        }
    }
}

/// Telegram said the token no longer works: stop, keep the token for the person to replace, and
/// turn Allow approvals off.
pub async fn rejected(state: &AppState, runner: &Runner) {
    let _ = state.db.telegram_set_state(&runner.space, db::REJECTED, Some("token_rejected")).await;
    super::stop(state, &runner.space);
    emit_status(state, &runner.space);
}

/// One update. Only a private chat's own text or a button tap is looked at.
pub async fn handle_update(state: &AppState, runner: &Runner, u: &Value) {
    if let Some(cb) = u.get("callback_query") {
        return handle_callback(state, runner, cb).await;
    }
    // Edits, channel posts, membership changes and the rest are ignored
    let Some(msg) = u.get("message") else { return };
    let Ok(Some(link)) = state.db.telegram_link(&runner.space).await else { return };
    let chat_id = msg["chat"]["id"].as_i64();
    let from_id = msg["from"]["id"].as_i64();
    let private = msg["chat"]["type"].as_str() == Some("private");
    let from_bot = msg["from"]["is_bot"].as_bool().unwrap_or(false);
    let (Some(chat_id), Some(from_id)) = (chat_id, from_id) else { return };
    let ours = link.is_paired() && link.user_id == Some(from_id) && link.chat_id == Some(chat_id);
    // Groups, channels, other bots and every other account: no reply, only a count (Q-G1)
    if !private || from_bot || (link.is_paired() && !ours) {
        let _ = state.db.telegram_count_stranger(&runner.space).await;
        return;
    }
    // Forwarded messages are someone else's words: ignored (Q-G1)
    if ["forward_origin", "forward_from", "forward_from_chat", "forward_date", "forward_sender_name"].iter().any(|k| msg.get(*k).is_some()) {
        return;
    }
    let text = msg["text"].as_str();
    if !ours {
        return pairing_message(state, runner, &link, msg, chat_id, from_id, text).await;
    }
    let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else {
        runner.enqueue(chat_id, plain(copy::TEXT_ONLY));
        return;
    };
    if is_command(text, "stop") {
        if state.db.telegram_unpair(&runner.space, None).await.ok().flatten().is_some() {
            runner.enqueue(chat_id, plain(copy::UNPAIRED));
            emit_status(state, &runner.space);
        }
        return;
    }
    if pairing::start_code(text).is_some() {
        let name = space_name(state, &runner.space).await;
        runner.enqueue(chat_id, html(copy::paired(&name)));
        return;
    }
    person_message(state, runner, &link, msg, chat_id, text).await;
}

/// A message from a chat that isn't paired: only `/start <code>` while a code is out can pair.
async fn pairing_message(state: &AppState, runner: &Runner, link: &TelegramLink, msg: &Value, chat_id: i64, from_id: i64, text: Option<&str>) {
    let code = text.and_then(pairing::start_code).filter(|c| !c.is_empty());
    let (Some(code), true) = (code, link.state == db::PAIRING) else {
        let _ = state.db.telegram_count_stranger(&runner.space).await;
        return;
    };
    // One sender's wrong codes lock out that sender only, so a stranger who knows the bot's name
    // can't keep the person from pairing (Lin's L1). Many senders together lock the space.
    if link.pair_locked || runner.sender_locked(from_id) {
        let _ = state.db.telegram_count_stranger(&runner.space).await;
        return;
    }
    let hash = pairing::code_hash(code);
    let live = link.pair_expires_at.is_some_and(|t| crate::clock::now() < t);
    let matches = link.pair_code_sha256.as_deref().is_some_and(|h| constant_eq(h.as_bytes(), hash.as_bytes()));
    if live && matches {
        let name = [msg["from"]["first_name"].as_str(), msg["from"]["last_name"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
        let name = if name.trim().is_empty() { "Telegram".to_string() } else { name.trim().to_string() };
        let username = msg["from"]["username"].as_str().filter(|u| u.len() <= 32 && u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        if state.db.telegram_pair(&runner.space, &hash, from_id, chat_id, &name, username).await.unwrap_or(false) {
            let _ = state.event_sender.send(RuntimeEvent::TelegramPaired { space_id: runner.space.clone(), name, username: username.map(str::to_string) });
            emit_status(state, &runner.space);
            let space = space_name(state, &runner.space).await;
            runner.enqueue(chat_id, html(copy::paired(&space)));
        }
        return;
    }
    runner.sender_failed(from_id);
    let window_start = crate::clock::now() - chrono::Duration::minutes(pairing::FAIL_WINDOW_MINUTES);
    let _ = state.db.telegram_code_failed(&runner.space, window_start, pairing::MAX_SPACE_FAILURES).await;
    emit_status(state, &runner.space);
    if runner.may_reply_bad_code(chat_id) {
        runner.enqueue(chat_id, plain(copy::BAD_CODE));
    }
}

/// The paired person wrote: an answer to a question they replied to, or a #general message.
async fn person_message(state: &AppState, runner: &Runner, link: &TelegramLink, msg: &Value, chat_id: i64, text: &str) {
    let space = runner.space.clone();
    let tg_id = msg["message_id"].as_i64().unwrap_or_default();
    let replied = msg["reply_to_message"]["message_id"].as_i64();
    // A reply to a question kwant sent answers it, choices or not
    if let Some(r) = replied {
        if let Some(ask) = state.db.telegram_open_asks(&space, "question", false).await.unwrap_or_default().into_iter().find(|a| a.message_id == Some(r)) {
            if state.db.telegram_close_ask(&ask.id).await.unwrap_or(false) {
                match crate::server::questions::answer_via(state, &ask.ref_id, text, Some("telegram")).await {
                    Some(_) => edit_answered(state, runner, &ask, &format!("{TEAL_CHECK} You answered · {}", hhmm(state).await)).await,
                    None => edit_answered(state, runner, &ask, &format!("{TEAL_CHECK} Answered in kwant · {}", hhmm(state).await)).await,
                }
                return;
            }
        }
    }
    let Ok(Some(general)) = state.db.general_channel(&SpaceId::new(space.clone())).await else {
        runner.enqueue(chat_id, plain(copy::NO_GENERAL));
        return;
    };
    let reply_to = match replied {
        Some(r) => state.db.telegram_message_for(&space, chat_id, r).await.ok().flatten(),
        None => None,
    };
    // Open the reply window before the message goes in, so a fast reply isn't missed
    let _ = state.db.telegram_person_wrote(&space, crate::clock::now()).await;
    let _ = link;
    let (prompt, unknown) = unknown_mentions(state, &space, text).await;
    let payload = crate::server::routes::SendChatPayload {
        channel_id: Some(general.id.clone()),
        kwant_id: None,
        team_id: None,
        thread_id: None,
        prompt,
        sender_id: "user".into(),
        attachment_ids: Vec::new(),
        reply_to_message_id: reply_to,
        source: Some("telegram".into()),
    };
    match crate::server::routes::send_chat_inner(state.clone(), payload, true).await {
        Ok(axum::Json(v)) => {
            if let Some(mid) = v["user_message_id"].as_str() {
                let _ = state.db.telegram_map_message(&space, chat_id, tg_id, mid).await;
            }
        }
        Err(code) => {
            tracing::info!(space = %space, "telegram: a message from Telegram wasn't taken: {code}");
            runner.enqueue(chat_id, plain(copy::NOT_SENT));
            return;
        }
    }
    if let Some(name) = unknown {
        let sname = space_name(state, &space).await;
        runner.enqueue(chat_id, html(copy::no_such_bot(&name, &sname)));
    }
}

/// `text` with the `@` taken off every `@Name` that names no bot or team in the space (kwant
/// refuses those), and the first such name, to tell the person. Same names as kwant's own
/// routing: a bot's or team's whole name, any case.
async fn unknown_mentions(state: &AppState, space: &str, text: &str) -> (String, Option<String>) {
    let sid = SpaceId::new(space);
    let kwants = state.db.list_kwants(&sid).await.unwrap_or_default();
    let teams = state.db.list_teams(&sid).await.unwrap_or_default();
    let known = |n: &str| kwants.iter().any(|k| k.name.eq_ignore_ascii_case(n)) || teams.iter().any(|t| t.name.eq_ignore_ascii_case(n));
    let unknown: Vec<String> = crate::router::tags::parse_mentions(text)
        .into_iter()
        .filter(|m| m.is_kwant() && !known(m.name()))
        .map(|m| m.name().to_string())
        .collect();
    let first = unknown.first().cloned();
    (drop_at(text, &unknown), first)
}

/// Removes the `@` in front of each of `names` (whole tokens, any case).
fn drop_at(text: &str, names: &[String]) -> String {
    if names.is_empty() {
        return text.to_string();
    }
    let token = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (i == 0 || !token(chars[i - 1])) {
            let end = (i + 1..chars.len()).find(|&j| !token(chars[j])).unwrap_or(chars.len());
            let word: String = chars[i + 1..end].iter().collect();
            if !word.is_empty() && names.iter().any(|n| n.eq_ignore_ascii_case(&word)) {
                out.push_str(&word);
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// A button tap. Only the paired person's taps on kwant's own messages do anything; a stale tap
/// says so and changes nothing (A21).
async fn handle_callback(state: &AppState, runner: &Runner, cb: &Value) {
    let Some(cb_id) = cb["id"].as_str() else { return };
    let Ok(Some(link)) = state.db.telegram_link(&runner.space).await else { return };
    let from = cb["from"]["id"].as_i64();
    let chat = cb["message"]["chat"]["id"].as_i64();
    let tg_msg = cb["message"]["message_id"].as_i64();
    if !link.is_paired() || from != link.user_id || chat != link.chat_id {
        let _ = state.db.telegram_count_stranger(&runner.space).await;
        let _ = runner.client.answer_callback(cb_id, None).await;
        return;
    }
    let data = cb["data"].as_str().unwrap_or_default();
    let mut parts = data.splitn(3, ':');
    let (kind, ask_id, n) = (parts.next(), parts.next(), parts.next());
    let ask = match ask_id {
        Some(id) => state.db.telegram_ask(&runner.space, id).await.ok().flatten(),
        None => None,
    };
    let Some(ask) = ask.filter(|a| a.message_id == tg_msg && a.state == "open") else {
        let _ = runner.client.answer_callback(cb_id, Some(copy::ALREADY_ANSWERED)).await;
        return;
    };
    // A tap counts only on its own kind of message: an answer on a question, Allow once on an
    // approval. A forged `callback_data` that crosses them is ignored (Lin's L5).
    match (kind, ask.kind.as_str()) {
        (Some("q"), "question") => {
            let choices: Vec<String> = serde_json::from_str(&ask.choices).unwrap_or_default();
            let Some(choice) = n.and_then(|n| n.parse::<usize>().ok()).and_then(|i| choices.get(i)).cloned() else {
                let _ = runner.client.answer_callback(cb_id, None).await;
                return;
            };
            if !state.db.telegram_close_ask(&ask.id).await.unwrap_or(false) {
                let _ = runner.client.answer_callback(cb_id, Some(copy::ALREADY_ANSWERED)).await;
                return;
            }
            let _ = runner.client.answer_callback(cb_id, None).await;
            let line = match crate::server::questions::answer_via(state, &ask.ref_id, &choice, Some("telegram")).await {
                Some(_) => format!("{TEAL_CHECK} You picked {} · {}", text::escape(&choice), hhmm(state).await),
                None => format!("{TEAL_CHECK} Answered in kwant · {}", hhmm(state).await),
            };
            edit_answered(state, runner, &ask, &line).await;
        }
        (Some("a"), "approval") => allow_once(state, runner, &link, &ask, cb_id).await,
        _ => {
            let _ = runner.client.answer_callback(cb_id, None).await;
        }
    }
}

/// Allow once from Telegram (Q-G6): only with the switch on, a kind on the list, no forcing
/// reason, and the card unchanged since it was sent. Anything else turns the message link-only.
async fn allow_once(state: &AppState, runner: &Runner, link: &TelegramLink, ask: &TelegramAsk, cb_id: &str) {
    let approval = state.db.get_approval(&ask.ref_id).await.ok().flatten();
    let details: Value = approval.as_ref().and_then(|a| serde_json::from_str(&a.parameters).ok()).unwrap_or(Value::Null);
    let sent_hash: Vec<String> = serde_json::from_str(&ask.choices).unwrap_or_default();
    let same = approval.as_ref().is_some_and(|a| sent_hash.first().is_some_and(|h| constant_eq(h.as_bytes(), params_hash(&a.parameters).as_bytes())));
    let pending = approval.as_ref().is_some_and(|a| a.status == "pending");
    if !pending {
        if state.db.telegram_close_ask(&ask.id).await.unwrap_or(false) {
            edit_answered(state, runner, ask, &format!("{TEAL_CHECK} Answered in kwant · {}", hhmm(state).await)).await;
        }
        let _ = runner.client.answer_callback(cb_id, Some(copy::ALREADY_ANSWERED)).await;
        return;
    }
    let why = super::link_only_in(state, &runner.space, &details).await;
    if !(ask.allow && link.allow_approvals && why.is_none() && same) {
        let _ = state.db.telegram_ask_link_only(&ask.id).await;
        let _ = runner.client.answer_callback(cb_id, Some(copy::APPROVE_IN_KWANT)).await;
        link_only(state, runner, ask, &details).await;
        return;
    }
    if !state.db.telegram_close_ask(&ask.id).await.unwrap_or(false) {
        let _ = runner.client.answer_callback(cb_id, Some(copy::ALREADY_ANSWERED)).await;
        return;
    }
    let (status, body) = crate::server::routes::resolve_approval_inner(state, &ask.ref_id, ApprovalDecision::AllowOnce, Some("telegram")).await;
    if status.is_success() {
        let _ = runner.client.answer_callback(cb_id, None).await;
        edit_answered(state, runner, ask, &format!("{TEAL_CHECK} Allowed once from Telegram · {}", hhmm(state).await)).await;
    } else if body["status"] == "needs_backup" {
        let _ = state.db.telegram_reopen_ask(&ask.id).await;
        let _ = state.db.telegram_ask_link_only(&ask.id).await;
        let _ = runner.client.answer_callback(cb_id, Some(copy::NEEDS_BACKUP)).await;
        let html = format!("{}\n\n<i>{}</i>", ask.text, text::escape(copy::NEEDS_BACKUP));
        edit(state, runner, ask, html, open_markup(state, link, &runner.space, None).await);
    } else {
        let _ = runner.client.answer_callback(cb_id, Some(copy::NO_LONGER_WAITING)).await;
        edit_answered(state, runner, ask, &format!("<i>{}</i>", text::escape(copy::NO_LONGER_WAITING))).await;
    }
}

// --- Outbound ------------------------------------------------------------------------------

/// Listens for questions, approvals, human steps and notifications, until the bus closes. One
/// per server (tests run several servers in a process, each with its own bus).
pub fn spawn_listener(state: AppState) {
    let mut rx = state.event_sender.subscribe();
    tokio::spawn(async move {
        loop {
            let event = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            on_event(&state, event.unscoped()).await;
        }
    });
}

async fn on_event(state: &AppState, event: &RuntimeEvent) {
    match event {
        RuntimeEvent::QuestionUpdated { question } => on_question(state, question).await,
        RuntimeEvent::ApprovalRequired { id, tool_name, details } => on_approval(state, id, tool_name, details).await,
        // An answered card's Telegram copy is edited by `approval_answered` (or the tap itself);
        // only a card nobody answered says it's no longer waiting
        RuntimeEvent::ApprovalWithdrawn { id, outcome, .. } if !RuntimeEvent::is_answer(outcome.as_deref()) => {
            approval_closed(state, id, &format!("<i>{}</i>", text::escape(copy::NO_LONGER_WAITING))).await
        }
        RuntimeEvent::HumanStepUpdated { step } => on_human_step(state, step).await,
        RuntimeEvent::Notify { kind, kwant_name, title, body, space_id: Some(space), channel_id, kwant_id, .. } => {
            on_notify(state, space, kind, kwant_name, title, body, channel_id.as_deref(), kwant_id).await
        }
        _ => {}
    }
}

/// The space's link and runner when it's paired, listening for this kind, and running.
async fn target(state: &AppState, space: &str, kind: &str) -> Option<(TelegramLink, Arc<Runner>, i64)> {
    let link = state.db.telegram_link(space).await.ok().flatten()?;
    if !link.is_paired() || !link.kind_on(kind) {
        return None;
    }
    let r = runner(state, space)?;
    let chat = link.chat_id?;
    Some((link, r, chat))
}

async fn on_question(state: &AppState, q: &Value) {
    let (Some(id), Some(status), Some(kwant)) = (q["id"].as_str(), q["status"].as_str(), q["kwant_id"].as_str()) else { return };
    let Some(space) = state.db.space_of_kwant(kwant).await.ok().flatten() else { return };
    let space = space.as_str().to_string();
    if status != "open" {
        let line = match (status, q["answered_via"].as_str()) {
            ("answered", Some("telegram")) => return,
            ("answered", _) => format!("{TEAL_CHECK} Answered in kwant · {}", hhmm(state).await),
            _ => format!("<i>{}</i>", text::escape(copy::NO_LONGER_WAITING)),
        };
        for ask in state.db.telegram_asks_for("question", id).await.unwrap_or_default() {
            if ask.state == "open" && state.db.telegram_close_ask(&ask.id).await.unwrap_or(false) {
                if let Some(r) = runner(state, &ask.space_id) {
                    edit_answered(state, &r, &ask, &line).await;
                }
            }
        }
        return;
    }
    let Some((link, r, chat)) = target(state, &space, "question").await else { return };
    if !state.db.telegram_asks_for("question", id).await.unwrap_or_default().is_empty() {
        return;
    }
    let who = kwant_name(state, kwant).await;
    let choices: Vec<String> = q["choices"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let body = format!("<b>{}</b> asks:\n{}", text::escape(&who), text::inline(q["question"].as_str().unwrap_or_default()));
    let ask_id = new_ask_id();
    let mut rows: Vec<Value> = choices
        .chunks(PER_ROW)
        .enumerate()
        .map(|(row, chunk)| {
            Value::Array(chunk.iter().enumerate().map(|(i, c)| json!({"text": clip(c, 60), "callback_data": format!("q:{ask_id}:{}", row * PER_ROW + i)})).collect())
        })
        .collect();
    if let Some(open) = open_row(&link, &space, q["channel_id"].as_str(), Some(kwant)) {
        rows.push(open);
    }
    send_ask(state, &r, chat, TelegramAsk {
        id: ask_id,
        space_id: space,
        kind: "question".into(),
        ref_id: id.to_string(),
        chat_id: chat,
        message_id: None,
        text: body,
        choices: serde_json::to_string(&choices).unwrap_or_else(|_| "[]".into()),
        allow: false,
        state: "open".into(),
        created_at: Utc::now(),
    }, rows)
    .await;
}

async fn on_approval(state: &AppState, id: &str, tool_name: &str, details: &Value) {
    let Some(space) = state.db.space_of_approval(id).await.ok().flatten() else { return };
    let space = space.as_str().to_string();
    let Some((link, r, chat)) = target(state, &space, "approval").await else { return };
    if !state.db.telegram_asks_for("approval", id).await.unwrap_or_default().is_empty() {
        return;
    }
    let Some(approval) = state.db.get_approval(id).await.ok().flatten() else { return };
    let who = details["kwant_name"].as_str().map(str::to_string).unwrap_or(kwant_name(state, &approval.kwant_id).await);
    // What is asked: the bot, the tool and where it runs. Never the arguments: a Telegram chat
    // isn't end-to-end encrypted, and they can hold anything the bot typed.
    let tool = details["tool"].as_str().unwrap_or(tool_name);
    let mut body = format!("<b>{}</b> wants to use <code>{}</code>", text::escape(&who), text::escape(&clip(tool, 80)));
    if let Some(server) = details["server"].as_str() {
        body.push_str(&format!(" on <code>{}</code>", text::escape(&clip(server, 60))));
    }
    body.push('.');
    let why = super::link_only_in(state, &space, details).await;
    let allow = link.allow_approvals && why.is_none();
    let ask_id = new_ask_id();
    let mut rows = Vec::new();
    if allow {
        rows.push(json!([{"text": copy::ALLOW_ONCE, "callback_data": format!("a:{ask_id}")}]));
    } else {
        body.push_str(&format!("\n\n<i>{}</i>", text::escape(&excluded_reason(why, link.allow_approvals))));
    }
    if let Some(open) = open_row(&link, &space, None, Some(&approval.kwant_id)) {
        rows.push(open);
    }
    send_ask(state, &r, chat, TelegramAsk {
        id: ask_id,
        space_id: space,
        kind: "approval".into(),
        ref_id: id.to_string(),
        chat_id: chat,
        message_id: None,
        text: body,
        // The card as it was sent: a tap allows only these exact parameters
        choices: json!([params_hash(&approval.parameters)]).to_string(),
        allow,
        state: "open".into(),
        created_at: Utc::now(),
    }, rows)
    .await;
}

/// Why an approval is link-only (A25): "… Approve it in kwant."
/// The line a link-only approval card carries: why, then "Approve it in kwant."
fn excluded_reason(why: Option<&str>, switch_on: bool) -> String {
    match (switch_on, why) {
        (true, Some(why)) => format!("{why} {}", copy::APPROVE_IN_KWANT),
        // Switch off, or a card that was armed and isn't any more (the switch or the card changed)
        _ => copy::APPROVE_IN_KWANT.into(),
    }
}

async fn on_human_step(state: &AppState, step: &Value) {
    let (Some(id), Some(status), Some(kwant)) = (step["id"].as_str(), step["status"].as_str(), step["kwant_id"].as_str()) else { return };
    if status != "open" {
        for ask in state.db.telegram_asks_for("alert", id).await.unwrap_or_default() {
            if ask.state == "open" && state.db.telegram_close_ask(&ask.id).await.unwrap_or(false) {
                if let Some(r) = runner(state, &ask.space_id) {
                    let line = if status == "done" { format!("{TEAL_CHECK} Done in kwant · {}", hhmm(state).await) } else { format!("<i>{}</i>", text::escape(copy::NO_LONGER_WAITING)) };
                    edit_answered(state, &r, &ask, &line).await;
                }
            }
        }
        return;
    }
    let Some(space) = state.db.space_of_kwant(kwant).await.ok().flatten() else { return };
    let space = space.as_str().to_string();
    let Some((link, r, chat)) = target(state, &space, "human_step").await else { return };
    if !state.db.telegram_asks_for("alert", id).await.unwrap_or_default().is_empty() {
        return;
    }
    let who = kwant_name(state, kwant).await;
    let what = match step["kind"].as_str() {
        Some("sign_in") => "needs you to sign in",
        Some("captcha") => "needs you to solve a CAPTCHA",
        Some("code") | Some("otp") => "needs a code from you",
        _ => "needs you for a step",
    };
    let body = format!("<b>{}</b> {what}.\n<i>{}</i>", text::escape(&who), text::escape(copy::SIGN_INS_IN_KWANT));
    let rows: Vec<Value> = open_row(&link, &space, step["channel_id"].as_str(), Some(kwant)).into_iter().collect();
    send_ask(state, &r, chat, alert_ask(&space, id, chat, body), rows).await;
}

#[allow(clippy::too_many_arguments)]
async fn on_notify(state: &AppState, space: &str, kind: &str, who: &str, title: &str, body: &str, channel: Option<&str>, kwant: &str) {
    let tg_kind = if STUCK_KINDS.contains(&kind) {
        "stuck"
    } else if DONE_KINDS.contains(&kind) {
        "done"
    } else {
        return;
    };
    let Some((link, r, chat)) = target(state, space, tg_kind).await else { return };
    // "A task you started from Telegram is done": only while the person is writing from there
    if tg_kind == "done" && link.reply_window_from.is_none() {
        return;
    }
    let head = if tg_kind == "done" { format!("Done: {}", text::escape(title)) } else { format!("<b>{}</b>", text::escape(title)) };
    let mut html = head;
    if !body.trim().is_empty() {
        html.push('\n');
        html.push_str(&text::escape(body));
    }
    let _ = who;
    let rows: Vec<Value> = open_row(&link, space, channel, (!kwant.is_empty()).then_some(kwant)).into_iter().collect();
    let markup = (!rows.is_empty()).then(|| json!({"inline_keyboard": rows}));
    r.enqueue(chat, Out::Send { parts: vec![html], markup, reply_to: None, ask_id: None, message_id: None });
}

fn alert_ask(space: &str, ref_id: &str, chat: i64, body: String) -> TelegramAsk {
    TelegramAsk {
        id: new_ask_id(),
        space_id: space.into(),
        kind: "alert".into(),
        ref_id: ref_id.into(),
        chat_id: chat,
        message_id: None,
        text: body,
        choices: "[]".into(),
        allow: false,
        state: "open".into(),
        created_at: Utc::now(),
    }
}

async fn send_ask(state: &AppState, r: &Runner, chat: i64, mut ask: TelegramAsk, rows: Vec<Value>) {
    // Room is kept for the "✓" line an answer adds
    ask.text = text::fit_one(&ask.text, 3900);
    if state.db.telegram_add_ask(&ask).await.is_err() {
        return;
    }
    let markup = (!rows.is_empty()).then(|| json!({"inline_keyboard": rows}));
    r.enqueue(chat, Out::Send { parts: vec![ask.text], markup, reply_to: None, ask_id: Some(ask.id), message_id: None });
}

/// A card was answered in kwant (or from Telegram): its Telegram copy loses its buttons.
pub async fn approval_answered(state: &AppState, id: &str, via: Option<&str>) {
    if via == Some("telegram") {
        return;
    }
    approval_closed(state, id, &format!("{TEAL_CHECK} Answered in kwant · {}", hhmm(state).await)).await;
}

async fn approval_closed(state: &AppState, id: &str, line: &str) {
    for ask in state.db.telegram_asks_for("approval", id).await.unwrap_or_default() {
        if ask.state == "open" && state.db.telegram_close_ask(&ask.id).await.unwrap_or(false) {
            if let Some(r) = runner(state, &ask.space_id) {
                edit_answered(state, &r, &ask, line).await;
            }
        }
    }
}

/// The person turned Allow approvals off: the switch goes off and every Allow once button still out
/// stops counting at once, in the database (`telegram_set_allow(false)` disarms them all). Their
/// messages are edited to link-only when kwant can reach Telegram; offline the edit is skipped, and
/// a tap is refused either way (Lin's M1).
pub async fn allow_off(state: &AppState, space: &str) {
    let armed = state.db.telegram_open_asks(space, "approval", true).await.unwrap_or_default();
    let _ = state.db.telegram_set_allow(space, false).await;
    let Some(r) = runner(state, space) else { return };
    for ask in armed {
        let details = match state.db.get_approval(&ask.ref_id).await.ok().flatten() {
            Some(a) => serde_json::from_str(&a.parameters).unwrap_or(Value::Null),
            None => Value::Null,
        };
        link_only(state, &r, &ask, &details).await;
    }
}

async fn link_only(state: &AppState, r: &Runner, ask: &TelegramAsk, details: &Value) {
    let Ok(Some(link)) = state.db.telegram_link(&ask.space_id).await else { return };
    let why = super::link_only_in(state, &ask.space_id, details).await;
    let html = format!("{}\n\n<i>{}</i>", ask.text, text::escape(&excluded_reason(why, link.allow_approvals)));
    let kwant = details["kwant_id"].as_str();
    edit(state, r, ask, html, open_markup(state, &link, &ask.space_id, kwant).await);
}

/// The message with its buttons removed and `line` added (A21).
async fn edit_answered(_state: &AppState, r: &Runner, ask: &TelegramAsk, line: &str) {
    let html = format!("{}\n\n{line}", ask.text);
    if let Some(m) = ask.message_id {
        r.enqueue(ask.chat_id, Out::Edit { message_id: m, html, markup: None });
    }
}

fn edit(_state: &AppState, r: &Runner, ask: &TelegramAsk, html: String, markup: Option<Value>) {
    if let Some(m) = ask.message_id {
        r.enqueue(ask.chat_id, Out::Edit { message_id: m, html, markup });
    }
}

async fn open_markup(_state: &AppState, link: &TelegramLink, space: &str, kwant: Option<&str>) -> Option<Value> {
    open_row(link, space, None, kwant).map(|row| json!({"inline_keyboard": [row]}))
}

/// "↗ Open in kwant" as a button row, when kwant's address is https (Telegram refuses others).
/// The link is the one push notifications open: `/?space=…&channel=…` or `&kwant=…`.
fn open_row(link: &TelegramLink, space: &str, channel: Option<&str>, kwant: Option<&str>) -> Option<Value> {
    let base = link.app_url.as_deref()?;
    let p = crate::server::push::Payload::new("telegram", "", "", Some(space.to_string()), channel.map(str::to_string), kwant.unwrap_or_default().to_string());
    Some(json!([{"text": format!("↗ {}", copy::OPEN_IN_KWANT), "url": format!("{}{}", base.trim_end_matches('/'), p.url)}]))
}

// --- The timers ----------------------------------------------------------------------------

/// Pairing expiry, the reply sweep and answered approvals, every couple of seconds.
pub async fn tick_loop(state: AppState, runner: Arc<Runner>) {
    loop {
        tokio::select! {
            _ = runner.cancel.cancelled() => return,
            _ = tokio::time::sleep(TICK) => {}
        }
        tick(&state, &runner).await;
    }
}

pub async fn tick(state: &AppState, runner: &Runner) {
    let Ok(Some(link)) = state.db.telegram_link(&runner.space).await else { return };
    let now = crate::clock::now();
    if link.state == db::PAIRING && link.pair_expires_at.is_some_and(|t| now >= t) {
        if let Some(h) = link.pair_code_sha256.as_deref() {
            if state.db.telegram_clear_code(&runner.space, h).await.unwrap_or(false) {
                let _ = state.event_sender.send(RuntimeEvent::TelegramExpired { space_id: runner.space.clone() });
                emit_status(state, &runner.space);
            }
        }
    }
    if !link.is_paired() {
        return;
    }
    sweep_replies(state, runner, &link, now).await;
}

/// A19: bot messages in #general since the cursor go to Telegram while the window is open. The
/// window closes after six hours, or when the person writes in kwant.
async fn sweep_replies(state: &AppState, runner: &Runner, link: &TelegramLink, now: DateTime<Utc>) {
    let Some(from) = link.reply_window_from else { return };
    if now - from > chrono::Duration::hours(REPLY_WINDOW_HOURS) {
        let _ = state.db.telegram_close_window(&runner.space).await;
        return;
    }
    let (Some(chat), Some(at)) = (link.chat_id, link.reply_cursor_at) else { return };
    let Ok(Some(general)) = state.db.general_channel(&SpaceId::new(runner.space.clone())).await else { return };
    let cursor_id = link.reply_cursor_id.clone().unwrap_or_default();
    let msgs = state.db.telegram_channel_since(&general.id, at, &cursor_id, 20).await.unwrap_or_default();
    for m in msgs {
        let created = m.created_at.unwrap_or(now);
        let _ = state.db.telegram_set_cursor(&runner.space, created, &m.id).await;
        let meta: Value = m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null);
        match m.sender_role.as_str() {
            "user" if meta["source"] == "telegram" => {}
            "user" => {
                // The person is back in kwant: replies stay there
                let _ = state.db.telegram_close_window(&runner.space).await;
                return;
            }
            "kwant" if !m.text.trim().is_empty() => {
                let who = kwant_name(state, &m.sender_id).await;
                let header = format!("<b>{}</b> · #{}", text::escape(&who), text::escape(&general.name));
                let parts = text::message_parts(Some(&header), &m.text);
                runner.enqueue(chat, Out::Send { parts, markup: None, reply_to: None, ask_id: None, message_id: Some(m.id.clone()) });
                let _ = state.db.telegram_mark_message(&m.id, "sent_to_telegram", &json!(true)).await;
            }
            _ => {}
        }
    }
}

// --- Small things --------------------------------------------------------------------------

fn plain(s: &str) -> Out {
    html(text::escape(s))
}

fn html(s: String) -> Out {
    Out::Send { parts: vec![s], markup: None, reply_to: None, ask_id: None, message_id: None }
}

/// `/name` or `/name@Bot`, alone.
fn is_command(text: &str, name: &str) -> bool {
    let t = text.trim();
    let Some(rest) = t.strip_prefix('/').and_then(|r| r.strip_prefix(name)) else { return false };
    rest.is_empty() || (rest.starts_with('@') && !rest.contains(char::is_whitespace))
}

fn new_ask_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

/// SHA-256 of a card's parameters as stored.
pub fn params_hash(parameters: &str) -> String {
    pairing::hex(&Sha256::digest(parameters.as_bytes()))
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

async fn kwant_name(state: &AppState, id: &str) -> String {
    state.db.get_kwant(id).await.ok().flatten().map(|k| k.name).unwrap_or_else(|| "A bot".into())
}

async fn space_name(state: &AppState, space: &str) -> String {
    state.db.get_space(&SpaceId::new(space)).await.ok().flatten().map(|s| s.name).unwrap_or_else(|| "kwant".into())
}

/// "10:06" in the person's time zone (UTC when none is set).
async fn hhmm(state: &AppState) -> String {
    let now = crate::clock::now();
    let tz = state.db.get_setting(crate::server::bot_actions::TIMEZONE_SETTING).await.ok().flatten().unwrap_or_default();
    let local = if tz.trim().is_empty() { None } else { crate::routines::local_datetime(&tz, now).ok().flatten() };
    local.unwrap_or_else(|| now.naive_utc()).format("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_names_lose_their_at_sign_only() {
        let n = vec!["Nobodyhere".to_string()];
        assert_eq!(drop_at("@Nobodyhere are you there", &n), "Nobodyhere are you there");
        assert_eq!(drop_at("hi @nobodyhere, @Drafter too", &n), "hi nobodyhere, @Drafter too");
        assert_eq!(drop_at("@Nobodyhere2 and me@Nobodyhere", &n), "@Nobodyhere2 and me@Nobodyhere");
        assert_eq!(drop_at("@Drafter", &[]), "@Drafter");
    }

    #[test]
    fn commands_alone_only() {
        assert!(is_command("/stop", "stop"));
        assert!(is_command(" /stop@kwant_bot ", "stop"));
        assert!(!is_command("/stopped", "stop"));
        assert!(!is_command("/stop now", "stop"));
        assert!(!is_command("please /stop", "stop"));
    }

    #[test]
    fn excluded_approvals_say_why_and_where() {
        let why = |d: Value| super::super::link_only_reason(&d);
        assert_eq!(excluded_reason(why(json!({"source": "local_shell"})), false), "Approve it in kwant.");
        assert_eq!(excluded_reason(why(json!({"source": "mcp", "prod_floor": {}})), true), "This is a production connection. Approve it in kwant.");
        assert_eq!(excluded_reason(why(json!({"source": "local_shell"})), true), "This is a command on your computer. Approve it in kwant.");
        assert_eq!(excluded_reason(None, true), "Approve it in kwant.");
    }

    #[test]
    fn constant_time_compare() {
        assert!(constant_eq(b"abc", b"abc"));
        assert!(!constant_eq(b"abc", b"abd"));
        assert!(!constant_eq(b"abc", b"ab"));
    }
}
