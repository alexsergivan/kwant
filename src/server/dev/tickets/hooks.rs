//! What Juno's poller calls (jira-tickets api.md 1.3), and where the brief's answers come in: a
//! question card, the brief's chat, Go with the defaults, or the reporter's reply on Jira.
//!
//! The reading turn (J6) is a turn of the team's lead in its own chat, "Prepare the brief for
//! KEY", spending on a held Mission Control task with its own token budget (J13). It reads only
//! (see [`super::tools`]) and ends with `submit_brief`. Then kwant asks the brief's questions as
//! one-tap cards in that chat (J8), or starts the work (J10, [`super::mission`]).

use super::brief::{self, Brief, JiraReply, DEFAULTS_ANSWER};
use super::seam::{self, Site};
use super::source::Comment;
use super::{mission, prompt, tools, valid_key, BRIEF_PURPOSE};
use crate::db::dev_ticket_briefs::{BriefRow, TicketQuestion};
use crate::db::missions::{NewCheck, NewTask, TaskChange};
use crate::db::models::{MessageExtra, NewMessage};
use crate::db::models::Kwant;
use crate::db::questions::{Question as Card, QuestionPlace, QuestionReason};
use crate::db::spaces::SpaceId;
use crate::server::missions::{self, Actor};
use crate::server::routes::{scope_for, turn_once, TurnOpts};
use crate::server::AppState;
use chrono::Utc;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

/// A ticket handed to a team (api.md 1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRef {
    pub site_id: String,
    pub key: String,
    pub team_id: String,
}

/// Why a ticket's work stops (J14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Unassigned, or the label removed.
    TakenBack,
    /// Done or deleted in Jira.
    Closed,
}

/// The brief turn's token budget when `KWANT_DEV_BRIEF_TOKENS` doesn't set one (J13).
pub const DEFAULT_BRIEF_TOKENS: i64 = 150_000;
/// The longest a brief turn runs.
const TURN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// The longest answer kept, in characters (api-decisions 13).
pub const MAX_ANSWER_CHARS: usize = 2000;
/// The longest Jira reply read as an answer, in characters.
const MAX_REPLY_CHARS: usize = 2000;

/// One answer at a time: an answer reads, changes and writes the brief.
static ANSWERS: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

/// The brief turn's token budget: `KWANT_DEV_BRIEF_TOKENS` (1000–2,000,000), or 150,000.
pub fn brief_budget() -> i64 {
    std::env::var("KWANT_DEV_BRIEF_TOKENS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|n| (1000..=2_000_000).contains(n))
        .unwrap_or(DEFAULT_BRIEF_TOKENS)
}

/// The brief's chat title.
pub fn thread_title(key: &str) -> String {
    format!("Prepare the brief for {key}")
}

fn parse(row: &BriefRow) -> Option<Brief> {
    serde_json::from_str(row.brief.as_deref()?).ok()
}

async fn row(state: &AppState, space_id: &str, key: &str) -> Option<BriefRow> {
    state.db.ticket_brief(space_id, key).await.ok().flatten()
}

/// The person's saved edits to the ticket's brief (api-decisions 14).
fn edits_of(r: &BriefRow) -> brief::Edits {
    r.person_edits.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default()
}

/// Saves the brief as kwant keeps it, and returns it: the person's edits on top (they win over
/// re-generated text), and every link off the site dropped (api-decisions 18).
async fn save(state: &AppState, space_id: &str, key: &str, b: &Brief) -> Brief {
    let mut b = b.clone();
    if let Some(r) = row(state, space_id, key).await {
        b = brief::with_edits(b, &edits_of(&r));
        let dropped = super::links::keep_site_links(&mut b, &seam::origins(state, space_id, &r.site_id).await);
        if dropped > 0 {
            tracing::warn!(%key, dropped, "dev: links off the Jira site were dropped from a brief");
        }
    }
    let json = serde_json::to_string(&b).unwrap_or_default();
    if let Err(e) = state.db.set_ticket_brief_field(space_id, key, "brief", Some(&json)).await {
        tracing::warn!(%key, error = %e, "dev: could not save a brief");
    }
    b
}

async fn fail(state: &AppState, space_id: &str, key: &str, note: &str) {
    let _ = state.db.set_ticket_brief_field(space_id, key, "note", Some(note)).await;
    seam::fail(state, space_id, key, super::store::FailCode::BriefFailed).await;
}

async fn open_questions(state: &AppState, space_id: &str, key: &str) -> Vec<TicketQuestion> {
    let mut qs: Vec<TicketQuestion> = state.db.ticket_questions(space_id, key).await.unwrap_or_default().into_iter().filter(|q| q.answer.is_none()).collect();
    qs.sort_by_key(|q| q.idx);
    qs
}

/// The ticket's turn has come (api.md 1.3). Only a ticket whose card is `picked_up` starts (the
/// card row is the one truth, D2). A ticket with a brief resumes from it (its open questions, or a
/// new mission, J14), whatever its brief row was left at when it was taken back, closed or failed
/// (D4); otherwise the lead reads it. Nothing happens for a ticket the board doesn't have.
pub async fn start(state: &AppState, space_id: &str, t: TicketRef) {
    if !valid_key(&t.key) {
        return;
    }
    let Ok(Some(card)) = state.db.dev_ticket(space_id, &t.key).await else { return };
    if card.state != crate::db::dev_jira::PICKED_UP {
        return;
    }
    let Some(site) = seam::site(state, space_id, &t.site_id).await else {
        // Its site can't be built (gone, or its token unreadable): fail it, so it doesn't hold a
        // slot in picked_up forever. Retry starts it again.
        tracing::warn!(key = %t.key, "dev: no site to read a ticket from");
        seam::fail(state, space_id, &t.key, super::store::FailCode::BriefFailed).await;
        return;
    };
    if let Some(r) = row(state, space_id, &t.key).await {
        // Handed over again: the brief row comes back with its card
        super::store::mirror(state, space_id, &t.key, crate::db::dev_jira::PICKED_UP).await;
        if let Some(b) = parse(&r) {
            resume(state, &site, space_id, &t.key, b).await;
            return;
        }
    }
    read(state, &site, space_id, &t).await;
}

/// A ticket back with its brief: its open questions again, or the work.
async fn resume(state: &AppState, site: &Site, space_id: &str, key: &str, b: Brief) {
    let _ = state.db.set_ticket_brief_field(space_id, key, "note", None).await;
    if !seam::set_state(state, space_id, key, "reading").await {
        return;
    }
    if !open_questions(state, space_id, key).await.is_empty() {
        seam::set_state(state, space_id, key, "needs_answer").await;
        return;
    }
    ready(state, site, space_id, key, &b).await;
}

/// The brief has nothing left to answer: Brief ready, then the work (J10).
async fn ready(state: &AppState, site: &Site, space_id: &str, key: &str, b: &Brief) {
    if !seam::set_state(state, space_id, key, "brief_ready").await {
        return;
    }
    if let Some(r) = row(state, space_id, key).await {
        mission::start(state, site, &r, b).await;
    }
}

/// A fresh read: the ticket pre-fetched, a chat and a held task for the lead, then the turn.
async fn read(state: &AppState, site: &Site, space_id: &str, t: &TicketRef) {
    let key = t.key.as_str();
    let (issue, comments) = match (site.source.issue(key).await, site.source.comments(key).await) {
        (Ok(i), Ok(c)) => (i, c),
        (Err(e), _) | (_, Err(e)) => {
            tracing::warn!(%key, error = %e, "dev: could not read a ticket");
            if state.db.upsert_ticket_brief(space_id, key, &t.site_id, &t.team_id, "", "").await.is_ok() {
                fail(state, space_id, key, "kwant couldn't read the ticket from Jira; press Start to try again.").await;
            }
            return;
        }
    };
    // The issue must be the one asked for: a source can't swap tickets
    if !issue.key.eq_ignore_ascii_case(key) {
        tracing::warn!(%key, got = %issue.key, "dev: the source returned another ticket");
        return;
    }
    if let Err(e) = state.db.upsert_ticket_brief(space_id, key, &t.site_id, &t.team_id, &super::links::keep(&issue.url, &site.origins), &issue.summary).await {
        tracing::warn!(%key, error = %e, "dev: could not save a ticket's brief row");
        return;
    }
    let _ = state.db.set_ticket_brief_field(space_id, key, "note", None).await;
    match state.db.take_dev_call(space_id).await {
        Ok(true) => {}
        _ => return fail(state, space_id, key, "Dev mode's daily model call cap is reached; press Start tomorrow.").await,
    }
    let team = match state.db.get_team(&t.team_id).await {
        Ok(Some(team)) if team.space_id == space_id => team,
        _ => return fail(state, space_id, key, "The team for this ticket is gone; pick another in Jira settings.").await,
    };
    let Ok(Some(lead)) = state.db.get_kwant(&team.lead_kwant_id).await else {
        return fail(state, space_id, key, "The team has no lead to read the ticket.").await;
    };
    let thread = match state.db.create_thread(&lead.id, &thread_title(key), "chat", Some(&team.id), "kwant").await {
        Ok(th) => th,
        Err(e) => {
            tracing::warn!(%key, error = %e, "dev: could not open the brief's chat");
            return fail(state, space_id, key, "kwant couldn't open the brief's chat; press Start to try again.").await;
        }
    };
    let task_id = hold_task(state, space_id, key, &lead, &thread.id).await;
    let _ = state.db.set_ticket_brief_field(space_id, key, "thread_id", Some(&thread.id)).await;
    let _ = state.db.set_ticket_brief_field(space_id, key, "task_id", task_id.as_deref()).await;
    let _ = state.db.set_ticket_brief_field(space_id, key, "brief", None).await;
    let _ = state.db.set_ticket_brief_field(space_id, key, "asked_comment_id", None).await;
    let _ = state.db.clear_ticket_questions(space_id, key).await;
    if !seam::set_state(state, space_id, key, "reading").await {
        close_task(state, task_id.as_deref(), false, "The ticket moved on before the brief started.").await;
        return;
    }
    let workspace = team.workspace_path.clone().filter(|w| !w.trim().is_empty());
    let mut session = tools::Session::new(space_id, &t.site_id, key, &lead.id, &thread.id, workspace.as_ref().map(PathBuf::from));
    session.task_id = task_id;
    session.known.origins = site.origins.clone();
    session.saw_ticket(&issue, &comments);
    tools::open(session);
    // Fixed words only: the event reaches every client of the space, and ticket text is untrusted (api.md 1.7)
    tools::progress(state, space_id, key, "ticket", "done", tools::ticket_detail(&issue));
    let n = comments.len();
    tools::progress(state, space_id, key, "comments", "done", &format!("{n} comment{}", if n == 1 { "" } else { "s" }));
    let text = prompt::brief_prompt(&issue, &comments, workspace.as_deref(), site.confluence);
    let (state, space_id, key) = (state.clone(), space_id.to_string(), key.to_string());
    tokio::spawn(async move {
        let result = run_turn(&state, &lead, &thread.id, &key, text).await;
        after_turn(&state, &space_id, &key, &thread.id, result.err()).await;
    });
}

/// The held task the brief turn spends on (J13): created for the lead and claimed by kwant, with
/// the brief's budget. Over it, the task is blocked and the brief tools stop.
async fn hold_task(state: &AppState, space_id: &str, key: &str, lead: &Kwant, thread_id: &str) -> Option<String> {
    let new = NewTask {
        mission_id: None,
        title: thread_title(key),
        acceptance: format!("A brief for {key} accepted by submit_brief: every line cited, at most {} blocking questions.", brief::MAX_QUESTIONS),
        state: "ready".into(),
        priority: 1,
        owner_kwant_id: None,
        suggested_kwant_id: Some(lead.id.clone()),
        budget_tokens: brief_budget(),
        max_attempts: missions::DEFAULT_MAX_ATTEMPTS,
        // The tasks table has no Dev mode kind; the source id says it's a ticket's brief
        source_kind: "chat".into(),
        source_id: Some(format!("dev_ticket:{space_id}:{key}")),
        channel_id: None,
        direct_kwant_id: None,
        thread_id: Some(thread_id.into()),
        created_by: "kwant".into(),
    };
    let task = match state.db.create_task(&SpaceId::new(space_id), &new).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(%key, error = %e, "dev: could not create the brief's task");
            return None;
        }
    };
    match state.db.claim_task(&task.id, &lead.id, None, Some(thread_id), missions::lease_ms(state).await, true, "kwant").await {
        Ok(Ok(t)) => missions::emit_task(state, &t).await,
        Ok(Err(why)) => tracing::info!(task_id = %task.id, %why, "dev: the brief's task wasn't claimed"),
        Err(e) => tracing::warn!(error = %e, "dev: could not claim the brief's task"),
    }
    Some(task.id)
}

/// The held task ends: verified (kwant's own check that a brief was accepted) or failed.
async fn close_task(state: &AppState, task_id: Option<&str>, ok: bool, why: &str) {
    let Some(id) = task_id else { return };
    let Ok(Some(task)) = state.db.get_task(id).await else { return };
    let change = || TaskChange { reason: Some(why.into()), ..Default::default() };
    match (ok, task.state.as_str()) {
        (true, "working") => {
            if missions::transition(state, id, "working", "review", Actor::Kwant, "kwant", change()).await.is_err() {
                return;
            }
            let check = NewCheck {
                task_id: id.into(),
                kind: "peer_review".into(),
                spec: serde_json::json!({"note": "kwant checked the brief: its schema and every source"}),
                required: true,
                approved: true,
                requested_by: "kwant".into(),
                approved_by: None,
            };
            if let Ok(c) = state.db.add_check(&check).await {
                let _ = state.db.finish_check(&c.id, "passed", Some("submit_brief accepted the brief"), "kwant", None).await;
            }
            missions::evaluate(state, id).await;
        }
        (false, "working") => {
            let _ = missions::transition(state, id, "working", "failed", Actor::Kwant, "kwant", change()).await;
        }
        (_, from) if matches!(from, "backlog" | "ready" | "needs_you" | "blocked" | "review") => {
            let _ = missions::transition(state, id, from, "cancelled", Actor::Kwant, "kwant", change()).await;
        }
        _ => {}
    }
    missions::emit_task_id(state, id).await;
}

async fn run_turn(state: &AppState, lead: &Kwant, thread_id: &str, key: &str, prompt: String) -> Result<String, String> {
    let job = state.turns.start_job([thread_id.to_string()]);
    let scope = scope_for(state, lead, None, Some(thread_id.to_string()), &job, 0).await?;
    let mut opts = TurnOpts::default();
    opts.purpose = Some(BRIEF_PURPOSE.into());
    opts.quiet_finish = true;
    tracing::info!(%key, kwant_id = %lead.id, "dev: reading a ticket for its brief");
    tokio::time::timeout(TURN_TIMEOUT, turn_once(&scope, lead, prompt, opts)).await.unwrap_or_else(|_| Err("the brief took longer than 15 minutes".into()))
}

/// The brief turn ended: its brief asks its questions or starts the work; without one, the card
/// fails and says why.
pub async fn after_turn(state: &AppState, space_id: &str, key: &str, thread_id: &str, error: Option<String>) {
    let session = tools::close(thread_id);
    let Some(r) = row(state, space_id, key).await else { return };
    if r.thread_id.as_deref() != Some(thread_id) {
        return;
    }
    let brief = session.as_ref().and_then(|s| s.brief.clone());
    if let Some(s) = &session {
        tools::progress_done(state, s, seam::confluence(state, space_id, &r.site_id).await.unwrap_or(false));
    }
    let lead_name = match r_lead(state, &r).await {
        Some(id) => state.db.get_kwant(&id).await.ok().flatten().map(|k| k.name).unwrap_or_default(),
        None => String::new(),
    };
    if r.state != "reading" {
        // Taken back or closed while reading: the brief is kept for a resume, nothing starts
        if let Some(b) = &brief {
            let mut b = brief::with_flags(b.clone(), brief::flags_for(&session.as_ref().map(|s| s.texts.clone()).unwrap_or_default()));
            brief::stamp(&mut b, &lead_name, Utc::now());
            save(state, space_id, key, &b).await;
        }
        close_task(state, r.task_id.as_deref(), false, "The ticket moved on while it was read.").await;
        return;
    }
    let Some(b) = brief else {
        let over = match r.task_id.as_deref() {
            Some(id) => state.db.get_task(id).await.ok().flatten().is_some_and(|t| !matches!(t.state.as_str(), "working" | "needs_you")),
            None => false,
        };
        let note = if over {
            "The brief went over its token budget; press Start to try again.".to_string()
        } else {
            match error {
                Some(e) => format!("The brief turn failed: {}", e.chars().take(200).collect::<String>()),
                None => "The bot ended without a brief; press Start to try again.".to_string(),
            }
        };
        close_task(state, r.task_id.as_deref(), false, &note).await;
        fail(state, space_id, key, &note).await;
        return;
    };
    let texts = session.map(|s| s.texts).unwrap_or_default();
    let mut b = brief::with_flags(b, brief::flags_for(&texts));
    brief::stamp(&mut b, &lead_name, Utc::now());
    close_task(state, r.task_id.as_deref(), true, "The brief is in.").await;
    if b.questions.is_empty() {
        let b = save(state, space_id, key, &b).await;
        if let Some(site) = seam::site(state, space_id, &r.site_id).await {
            ready(state, &site, space_id, key, &b).await;
        } else {
            seam::set_state(state, space_id, key, "brief_ready").await;
        }
        return;
    }
    let Ok(Some(lead)) = state.db.get_kwant(&r_lead(state, &r).await.unwrap_or_default()).await else {
        save(state, space_id, key, &b).await;
        return fail(state, space_id, key, "The team has no lead to ask the questions.").await;
    };
    ask(state, &lead, &r, thread_id, &mut b).await;
    save(state, space_id, key, &b).await;
    seam::set_state(state, space_id, key, "needs_answer").await;
}

async fn r_lead(state: &AppState, r: &BriefRow) -> Option<String> {
    state.db.get_team(&r.team_id).await.ok().flatten().map(|t| t.lead_kwant_id)
}

/// Asks each of the brief's questions as a one-tap card in its chat (J8), remembered so the
/// answer comes back to the brief whichever way it's given.
async fn ask(state: &AppState, lead: &Kwant, r: &BriefRow, thread_id: &str, b: &mut Brief) {
    let place = QuestionPlace { kwant_id: lead.id.clone(), thread_id: Some(thread_id.into()), ..QuestionPlace::default() };
    for (idx, q) in b.questions.iter_mut().enumerate() {
        let mut checked = String::new();
        if !q.conflict_sources.is_empty() {
            let names: Vec<&str> = q.conflict_sources.iter().map(|s| s.label.as_str()).collect();
            checked.push_str(&format!("The sources disagree: {}. ", names.join(" and ")));
        }
        checked.push_str(&format!("Blocks because: {} Default: {}, because {}.", q.why_blocks, q.default, q.default_reason));
        let reason = QuestionReason { why_you: Some("missing_after_research"), checked: Some(&checked) };
        let text = format!("{}: {}", r.key, q.text);
        let Some(card) = crate::server::questions::ask_without_waiter(state, &place, &lead.name, &text, &q.choices, reason).await else {
            tracing::warn!(key = %r.key, "dev: could not ask a brief question");
            continue;
        };
        if let Err(e) = state.db.add_ticket_question(&card.id, &r.space_id, &r.key, idx as i64, &q.default).await {
            tracing::warn!(key = %r.key, error = %e, "dev: could not remember a brief question");
        }
        q.card_id = Some(card.id);
    }
}

/// Who answered on Jira: their name and the reply's link.
type JiraWho<'a> = Option<(Option<&'a str>, &'a str)>;

/// One answer to one of the brief's questions: kept on the question (who, where, when), and when
/// none is left open the brief is ready. `by` is "card", "chat", "defaults", "dismissed" or
/// "jira"; `place` where it was given: "card", "chat" or "jira". Whether it was taken (false:
/// already answered, or not a brief question). The first answer wins, here or on Jira.
async fn take_answer(state: &AppState, question_id: &str, answer: &str, by: &str, place: &str, jira: JiraWho<'_>) -> bool {
    let _one = ANSWERS.lock().await;
    let Ok(Some(tq)) = state.db.ticket_question(question_id).await else { return false };
    let answer: String = answer.trim().chars().take(MAX_ANSWER_CHARS).collect();
    let answer = if answer.is_empty() { tq.default_answer.clone() } else { answer };
    if !state.db.answer_ticket_question(question_id, &answer, by).await.unwrap_or(false) {
        return false;
    }
    let Some(r) = row(state, &tq.space_id, &tq.key).await else { return true };
    let Some(mut b) = parse(&r) else { return true };
    let a = brief::Answer {
        value: answer.clone(),
        by: if by == "jira" { "jira_user" } else { "person" }.into(),
        name: jira.and_then(|(n, _)| n).map(|n| n.chars().take(100).collect()),
        place: place.into(),
        at: brief::at(Utc::now()),
        comment_url: jira.map(|(_, url)| url.to_string()),
        went_with_default: matches!(by, "defaults" | "dismissed"),
    };
    if brief::set_answer(&mut b, question_id, a) {
        save(state, &tq.space_id, &tq.key, &b).await;
    }
    tracing::info!(key = %tq.key, %by, "dev: a brief question was answered");
    if r.state == "needs_answer" && open_questions(state, &tq.space_id, &tq.key).await.is_empty() {
        if let Some(site) = seam::site(state, &tq.space_id, &r.site_id).await {
            ready(state, &site, &tq.space_id, &tq.key, &b).await;
        } else {
            seam::set_state(state, &tq.space_id, &tq.key, "brief_ready").await;
        }
    }
    true
}

/// Juno's `POST tickets/:key/ask` posted the open questions to Jira (api-decisions 16), after
/// `set_dev_ticket_asked`: each open question records when, where (`comment_url`, kept only when
/// it is on the site, api-decisions 18) and to whom. The reporter's reply to that comment then
/// comes back through [`reporter_replied`].
pub async fn asked_on_jira(state: &AppState, space_id: &str, key: &str, comment_url: &str, to: Option<&str>) {
    let _one = ANSWERS.lock().await;
    let Some(r) = row(state, space_id, key).await else { return };
    let Some(mut b) = parse(&r) else { return };
    let open = open_questions(state, space_id, key).await;
    let asked = brief::AskedOnJira {
        at: brief::at(Utc::now()),
        comment_url: super::links::keep(comment_url, &seam::origins(state, space_id, &r.site_id).await),
        to: to.map(|n| n.split_whitespace().collect::<Vec<_>>().join(" ").chars().filter(|c| !c.is_control()).take(80).collect()).unwrap_or_default(),
    };
    for q in b.questions.iter_mut().filter(|q| q.answer.is_none() && q.card_id.as_ref().is_some_and(|id| open.iter().any(|o| &o.question_id == id))) {
        q.asked_on_jira = Some(asked.clone());
    }
    save(state, space_id, key, &b).await;
}

/// Why the person's answer wasn't taken (`POST tickets/:key/answer`).
#[derive(Debug, PartialEq)]
pub enum AnswerRefused {
    /// Not one of this ticket's questions.
    NoQuestion,
    /// Answered already, here or on Jira: the first answer wins.
    Answered,
    /// The brief isn't waiting for answers.
    NotWaiting,
}

/// The person answered one of the brief's questions from the card (api-decisions 13): `value` is
/// already checked (a choice, or text of at most [`MAX_ANSWER_CHARS`]). The question card closes
/// with it; the last open one moves the ticket to Brief ready.
pub async fn answer(state: &AppState, space_id: &str, key: &str, question_id: &str, value: &str) -> Result<(), AnswerRefused> {
    let tq = match state.db.ticket_question(question_id).await {
        Ok(Some(q)) if q.space_id == space_id && q.key == key => q,
        _ => return Err(AnswerRefused::NoQuestion),
    };
    if tq.answer.is_some() {
        return Err(AnswerRefused::Answered);
    }
    if row(state, space_id, key).await.is_none_or(|r| r.state != "needs_answer") {
        return Err(AnswerRefused::NotWaiting);
    }
    let _ = crate::server::questions::close_answered(state, question_id, value).await;
    if !take_answer(state, question_id, value, "card", "card", None).await {
        return Err(AnswerRefused::Answered);
    }
    Ok(())
}

/// Why the person's edit wasn't taken (`PUT tickets/:key/brief`).
#[derive(Debug, PartialEq)]
pub enum EditRefused {
    /// No brief yet, or the work has started: edit while it waits for answers or for the OK.
    NotEditable,
}

/// The person edited the brief (api-decisions 14): kept apart from it, so they win over a brief
/// read again, and put on it now with `edited_by_person_at`. A draft mission waiting for the OK
/// is made again from the edited brief. The brief as it now reads.
pub async fn edit(state: &AppState, space_id: &str, key: &str, edits: brief::Edits) -> Result<Brief, EditRefused> {
    let _one = ANSWERS.lock().await;
    let Some(r) = row(state, space_id, key).await else { return Err(EditRefused::NotEditable) };
    let Some(b) = parse(&r) else { return Err(EditRefused::NotEditable) };
    if !matches!(r.state.as_str(), "needs_answer" | "brief_ready") {
        return Err(EditRefused::NotEditable);
    }
    let draft = match &r.mission_id {
        Some(id) => match state.db.get_mission(id).await.ok().flatten() {
            Some(m) if m.status == "draft" => true,
            Some(m) if m.status == "cancelled" => false,
            Some(_) => return Err(EditRefused::NotEditable),
            None => false,
        },
        None => false,
    };
    let edits = brief::Edits { at: Some(brief::at(Utc::now())), ..edits }.over(edits_of(&r));
    let json = serde_json::to_string(&edits).unwrap_or_default();
    if let Err(e) = state.db.set_ticket_brief_field(space_id, key, "person_edits", Some(&json)).await {
        tracing::warn!(%key, error = %e, "dev: could not save the person's edits");
        return Err(EditRefused::NotEditable);
    }
    let b = save(state, space_id, key, &b).await;
    tracing::info!(%key, "dev: the person edited a brief");
    if draft {
        // The draft was made from the old words: make it again from these
        mission::stop(state, &r, "The person edited the brief.").await;
        let _ = state.db.set_ticket_brief_field(space_id, key, "mission_id", None).await;
        if let (Some(site), Some(r)) = (seam::site(state, space_id, &r.site_id).await, row(state, space_id, key).await) {
            mission::start(state, &site, &r, &b).await;
        }
    }
    Ok(b)
}

/// A question card was answered (`answer`) or dismissed (`None`: its default counts). Whether it
/// was one of a brief's questions; then the brief took it and nothing else should.
pub async fn on_question(state: &AppState, q: &Card, answer: Option<&str>, by: &str) -> bool {
    if state.db.ticket_question(&q.id).await.ok().flatten().is_none() {
        return false;
    }
    let by = if answer.is_none() { "dismissed" } else { by };
    take_answer(state, &q.id, answer.unwrap_or(""), by, "card", None).await;
    true
}

/// Answers every open question of the ticket with its default (Go with the defaults). How many.
pub async fn defaults(state: &AppState, space_id: &str, key: &str) -> usize {
    let mut n = 0;
    for q in open_questions(state, space_id, key).await {
        let _ = crate::server::questions::close_answered(state, &q.question_id, &q.default_answer).await;
        if take_answer(state, &q.question_id, &q.default_answer, "defaults", "card", None).await {
            n += 1;
        }
    }
    n
}

/// Whether a message is "Go with the defaults".
pub fn says_defaults(text: &str) -> bool {
    let bare = text.trim().trim_end_matches(['.', '!']).trim();
    bare.eq_ignore_ascii_case(DEFAULTS_ANSWER) || bare.eq_ignore_ascii_case("defaults")
}

/// How a message answers the open questions, in order of `open`: `Some(vec)` of (index in
/// `open`, answer), `None` when it doesn't read as answers (then it's an ordinary message).
/// "Go with the defaults" answers all; with one open question the whole message is its answer;
/// with more, a choice of exactly one question, or numbered lines ("1. CSV", "2) yes").
pub fn read_answers(text: &str, open: &[(Vec<String>, String)]) -> Option<Vec<(usize, String)>> {
    let t = text.trim();
    if t.is_empty() || open.is_empty() {
        return None;
    }
    let bare = t.trim_end_matches(['.', '!']).trim();
    if says_defaults(t) {
        return Some(open.iter().enumerate().map(|(i, (_, d))| (i, d.clone())).collect());
    }
    if open.len() == 1 {
        return Some(vec![(0, t.to_string())]);
    }
    let picked: Vec<usize> = open.iter().enumerate().filter(|(_, (choices, _))| choices.iter().any(|c| c.trim().eq_ignore_ascii_case(bare))).map(|(i, _)| i).collect();
    if picked.len() == 1 {
        return Some(vec![(picked[0], t.to_string())]);
    }
    let out = numbered(t, open.len());
    (!out.is_empty()).then_some(out)
}

/// Numbered answer lines ("1. CSV", "2) yes", "3: no") for `n` questions: (index, answer), each
/// question once.
fn numbered(text: &str, n: usize) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
        let Ok(i) = digits.parse::<usize>() else { continue };
        let rest = line[digits.len()..].trim_start();
        let Some(rest) = rest.strip_prefix(['.', ')', ':']) else { continue };
        let answer = rest.trim();
        if i == 0 || i > n || answer.is_empty() || out.iter().any(|(j, _)| *j == i - 1) {
            continue;
        }
        out.push((i - 1, answer.to_string()));
    }
    out
}

/// Whether `word` is in `text` as a whole word (both lower-case).
fn has_word(text: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(i) = text[from..].find(word) {
        let start = from + i;
        let end = start + word.len();
        let before = text[..start].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = text[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if before && after {
            return true;
        }
        from = start + text[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// The one choice `text` picks: the choice itself, or the only choice it names.
fn pick(text: &str, choices: &[String]) -> Option<String> {
    let bare = text.trim().trim_end_matches(['.', '!']).trim();
    if let Some(c) = choices.iter().find(|c| c.trim().eq_ignore_ascii_case(bare)) {
        return Some(c.clone());
    }
    let low = text.to_lowercase();
    let hits: Vec<&String> = choices.iter().filter(|c| has_word(&low, &c.trim().to_lowercase())).collect();
    (hits.len() == 1).then(|| hits[0].clone())
}

/// How a Jira reply answers the open questions (their choices, in order): only when it maps to
/// choices, and then the answer is the choice as the question has it. One open question: the
/// reply is a choice or names exactly one. More: numbered lines, each picking a choice of its
/// question, or a reply that is a choice of exactly one question. Anything else (`None`) is shown
/// quoted, and the person confirms the answer (design.md 05).
pub fn reply_answers(text: &str, open: &[Vec<String>]) -> Option<Vec<(usize, String)>> {
    let t = text.trim();
    if t.is_empty() || open.is_empty() {
        return None;
    }
    if open.len() == 1 {
        return pick(t, &open[0]).map(|c| vec![(0, c)]);
    }
    let lines = numbered(t, open.len());
    if !lines.is_empty() {
        let out: Option<Vec<(usize, String)>> = lines.into_iter().map(|(i, a)| pick(&a, &open[i]).map(|c| (i, c))).collect();
        return out;
    }
    let bare = t.trim_end_matches(['.', '!']).trim();
    let picked: Vec<(usize, String)> =
        open.iter().enumerate().filter_map(|(i, choices)| choices.iter().find(|c| c.trim().eq_ignore_ascii_case(bare)).map(|c| (i, c.clone()))).collect();
    (picked.len() == 1).then_some(picked)
}

/// The choices of each open question, in the order of `open`.
fn choices_of(b: &Brief, open: &[TicketQuestion]) -> Vec<Vec<String>> {
    open.iter().map(|tq| b.questions.iter().find(|q| q.card_id.as_deref() == Some(&tq.question_id)).map(|q| q.choices.clone()).unwrap_or_default()).collect()
}

async fn answer_from_text(state: &AppState, r: &BriefRow, text: &str) -> bool {
    let open = open_questions(state, &r.space_id, &r.key).await;
    let b = parse(r).unwrap_or_default();
    let shapes: Vec<(Vec<String>, String)> = choices_of(&b, &open).into_iter().zip(open.iter().map(|tq| tq.default_answer.clone())).collect();
    let Some(answers) = read_answers(text, &shapes) else { return false };
    let all_defaults = says_defaults(text);
    for (i, answer) in answers {
        let q = &open[i];
        let by = if all_defaults { "defaults" } else { "chat" };
        let _ = crate::server::questions::close_answered(state, &q.question_id, &answer).await;
        take_answer(state, &q.question_id, &answer, by, "chat", None).await;
    }
    true
}

/// The person wrote in the brief's chat while it waits for answers: when the message reads as
/// answers (see [`read_answers`]) the brief takes them and no turn runs. Whether it did.
pub async fn answer_in_chat(state: &AppState, thread_id: &str, text: &str) -> bool {
    let Ok(Some(r)) = state.db.ticket_brief_by_thread(thread_id).await else { return false };
    if r.state != "needs_answer" || seam::confluence(state, &r.space_id, &r.site_id).await.is_none() {
        return false;
    }
    answer_from_text(state, &r, text).await
}

/// The reporter replied on Jira after the Ask on Jira comment (api.md 1.3). The reply is
/// untrusted: text in it that tries to steer the bot is flagged, and it answers only when it maps
/// to the questions' choices ([`reply_answers`]). Otherwise it is kept on each open question,
/// shown quoted on the card, and the brief's chat asks the person to confirm an answer.
pub async fn reporter_replied(state: &AppState, space_id: &str, key: &str, asked_comment_id: &str, reply: &Comment) {
    let Some(r) = row(state, space_id, key).await else { return };
    if r.state != "needs_answer" || r.asked_comment_id.as_deref() != Some(asked_comment_id) || reply.id == asked_comment_id {
        return;
    }
    let body: String = reply.body.trim().chars().take(MAX_REPLY_CHARS).collect();
    if body.is_empty() {
        return;
    }
    let name = reply.author.as_ref().map(|p| p.display_name.clone()).filter(|n| !n.trim().is_empty());
    let origins = seam::origins(state, space_id, &r.site_id).await;
    let ticket_url = super::links::keep(&r.url, &origins);
    let url = brief::comment_url(&ticket_url, &reply.id);
    let known = brief::Known { key: key.into(), ticket_url, comments: [(reply.id.clone(), name.clone())].into(), origins, ..Default::default() };
    let source = known.comment(&reply.id).unwrap_or_else(|| known.ticket());
    let found = brief::flags_for(&[(source, body.clone())]);
    let flagged = !found.is_empty();
    if flagged {
        let _one = ANSWERS.lock().await;
        if let Some(b) = row(state, space_id, key).await.as_ref().and_then(parse) {
            save(state, space_id, key, &brief::with_flags(b, found)).await;
        }
    }
    let Some(r) = row(state, space_id, key).await else { return };
    let open = open_questions(state, space_id, key).await;
    let b = parse(&r).unwrap_or_default();
    if let Some(answers) = reply_answers(&body, &choices_of(&b, &open)) {
        for (i, value) in answers {
            let q = &open[i];
            let _ = crate::server::questions::close_answered(state, &q.question_id, &value).await;
            take_answer(state, &q.question_id, &value, "jira", "jira", Some((name.as_deref(), &url))).await;
        }
        return;
    }
    // Not a choice: kept, quoted, for the person to confirm
    {
        let _one = ANSWERS.lock().await;
        let Some(mut b) = row(state, space_id, key).await.as_ref().and_then(parse) else { return };
        let reply = JiraReply { text: body.chars().take(brief::MAX_QUOTE_CHARS).collect(), name: name.clone(), at: brief::at(Utc::now()), comment_url: url.clone() };
        for q in b.questions.iter_mut().filter(|q| q.answer.is_none() && q.card_id.as_ref().is_some_and(|id| open.iter().any(|o| &o.question_id == id))) {
            q.jira_reply = Some(reply.clone());
        }
        save(state, space_id, key, &b).await;
    }
    let who = name.as_deref().map(|n| n.chars().take(80).collect::<String>()).unwrap_or_else(|| "The reporter".into());
    let text = if flagged {
        format!("{who} replied on Jira, but kwant can't match the reply to a choice, and part of it reads like an order to a bot (it was ignored). Pick an answer on the question card, or reply here.")
    } else {
        format!("{who} replied on Jira, but kwant can't match the reply to a choice. Their reply is quoted on the brief. Pick an answer on the question card, or reply here.")
    };
    if let (Some(thread), Some(lead)) = (r.thread_id.as_deref(), r_lead(state, &r).await) {
        chat_line(state, thread, &lead, key, text).await;
    }
}

/// A short line from kwant in the brief's chat (`meta.kind = "dev_ticket"`).
async fn chat_line(state: &AppState, thread_id: &str, sender: &str, key: &str, text: String) {
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: None,
        sender_role: "kwant".into(),
        sender_id: sender.into(),
        text,
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let extra = MessageExtra { thread_id: Some(thread_id.into()), meta: Some(serde_json::json!({"kind": "dev_ticket", "key": key})) };
    let Ok(saved) = state.db.insert_message_with(&msg, &extra).await else { return };
    let _ = state.db.touch_thread(thread_id).await;
    let _ = state.event_sender.send(crate::drivers::events::RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or_default() });
}

/// The ticket was taken back or closed in Jira (api.md 1.3, J14): its work stops (the mission
/// and its open tasks are cancelled). The brief, its chat and its answers stay; taken back and
/// handed over again, it resumes from the brief.
pub async fn stopped(state: &AppState, space_id: &str, key: &str, why: Why) {
    let Some(r) = row(state, space_id, key).await else { return };
    let text = match why {
        Why::TakenBack => "Taken back in Jira",
        Why::Closed => "Closed in Jira",
    };
    let _ = state.db.set_ticket_brief_field(space_id, key, "note", Some(text)).await;
    // The card moved first (pickup or the person's Stop/Close); the brief row stays where it went
    // (D4), so a late brief turn can't move it on, and a later Start resumes from the kept brief.
    if let Ok(Some(card)) = state.db.dev_ticket(space_id, key).await {
        super::store::mirror(state, space_id, key, &card.state).await;
    }
    mission::stop(state, &r, text).await;
    if why == Why::Closed {
        // Its questions won't be answered: their cards close with their defaults, which start nothing
        for q in open_questions(state, space_id, key).await {
            let _ = crate::server::questions::close_answered(state, &q.question_id, &q.default_answer).await;
            let _ = state.db.answer_ticket_question(&q.question_id, &q.default_answer, "dismissed").await;
        }
    }
    tracing::info!(%key, why = text, "dev: a ticket's work stopped");
}

/// What the ticket row's `work` object needs from the brief side (api-decisions 11, api.md 1.3):
/// the ticket's mission, and the Matches the ticket verdict.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkSeam {
    pub mission_id: Option<String>,
    pub matches: Option<String>,
}

/// One indexed read of the brief row. `matches` is only ever `met`, `not_met` or `unclear`.
pub async fn work_seam(state: &AppState, space_id: &str, key: &str) -> WorkSeam {
    let Ok(Some(r)) = state.db.ticket_brief(space_id, key).await else { return WorkSeam::default() };
    WorkSeam { mission_id: r.mission_id, matches: r.matches.filter(|v| matches!(v.as_str(), "met" | "not_met" | "unclear")) }
}

/// A review of the ticket's PR gave its "Matches the ticket" verdict (J12): the newest one is the
/// row's `work.matches`.
pub async fn reviewed(state: &AppState, space_id: &str, key: &str, verdict: &str) {
    if !matches!(verdict, "met" | "not_met" | "unclear") || !valid_key(key) {
        return;
    }
    if let Err(e) = state.db.set_ticket_brief_field(space_id, key, "matches", Some(verdict)).await {
        tracing::warn!(key, "tickets: could not keep the review's verdict: {e}");
    }
}

/// The brief's routes join Juno's Jira route table here (api.md 1.3), on the same person-only
/// footing: every handler checks `brief_routes::guard` (person only) itself. Paths are the full
/// `/api/spaces/:id/dev/tickets/:key/…`.
pub fn mount(r: axum::Router<AppState>) -> axum::Router<AppState> {
    use super::brief_routes as b;
    use axum::routing::{get, post};
    r.route("/api/spaces/:id/dev/tickets/:key/brief", get(b::get_brief).put(b::put_brief))
        .route("/api/spaces/:id/dev/tickets/:key/answer", post(b::answer))
        .route("/api/spaces/:id/dev/tickets/:key/defaults", post(b::defaults))
        .route("/api/spaces/:id/dev/tickets/:key/ok", post(b::ok))
}

/// A Jira site was deleted from the space (api.md 1.7, brief.md §10). Juno's `DELETE …/dev/jira`
/// calls this after the ticket rows are gone: the site's briefs and their questions go too.
/// Missions and chats the briefs started stay.
pub async fn site_deleted(state: &AppState, space_id: &str, site_id: &str) {
    match brief::forget_site(&state.db, space_id, site_id).await {
        Ok(n) => tracing::info!(%site_id, briefs = n, "dev: a Jira site's briefs were forgotten"),
        Err(e) => tracing::warn!(%site_id, error = %e, "dev: couldn't forget a Jira site's briefs"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(n: usize) -> Vec<(Vec<String>, String)> {
        (0..n).map(|i| (vec![format!("A{i}"), format!("B{i}")], format!("A{i}"))).collect()
    }

    #[test]
    fn chat_answers_read_the_way_people_write_them() {
        assert_eq!(read_answers("Go with the defaults", &open(2)), Some(vec![(0, "A0".into()), (1, "A1".into())]));
        assert_eq!(read_answers("go with the defaults.", &open(1)), Some(vec![(0, "A0".into())]));
        assert_eq!(read_answers("XLSX please", &open(1)), Some(vec![(0, "XLSX please".into())]), "one open question: the whole message");
        assert_eq!(read_answers("b1", &open(2)), Some(vec![(1, "b1".into())]), "a choice of exactly one question");
        assert_eq!(read_answers("1. CSV\n2) no\n2: again\n9. out of range", &open(2)), Some(vec![(0, "CSV".into()), (1, "no".into())]));
        assert_eq!(read_answers("What do you think?", &open(2)), None, "not answers: an ordinary message");
        assert_eq!(read_answers("   ", &open(1)), None);
        assert_eq!(read_answers("yes", &[]), None);
    }

    #[test]
    fn a_jira_reply_answers_only_when_it_maps_to_a_choice() {
        let one = vec![vec!["CSV".to_string(), "XLSX".to_string()]];
        assert_eq!(reply_answers("CSV", &one), Some(vec![(0, "CSV".into())]));
        assert_eq!(reply_answers("csv please, thanks!", &one), Some(vec![(0, "CSV".into())]), "names exactly one choice");
        assert_eq!(reply_answers("Either CSV or XLSX is fine", &one), None, "two choices: the person confirms");
        assert_eq!(reply_answers("Ask Sam", &one), None);
        assert_eq!(reply_answers("csvs", &one), None, "a whole word only");
        let two = vec![vec!["CSV".to_string(), "XLSX".to_string()], vec!["Everyone".to_string(), "Only admins".to_string()]];
        assert_eq!(reply_answers("1. csv\n2) only admins", &two), Some(vec![(0, "CSV".into()), (1, "Only admins".into())]));
        assert_eq!(reply_answers("1. csv\n2) ask Sam", &two), None, "every line must map");
        assert_eq!(reply_answers("Everyone", &two), Some(vec![(1, "Everyone".into())]));
        assert_eq!(reply_answers("CSV", &two), Some(vec![(0, "CSV".into())]));
        assert_eq!(reply_answers("", &one), None);
    }

    #[test]
    fn the_budget_comes_from_the_env_within_limits() {
        // Process-wide: only this test reads it
        std::env::remove_var("KWANT_DEV_BRIEF_TOKENS");
        assert_eq!(brief_budget(), DEFAULT_BRIEF_TOKENS);
        std::env::set_var("KWANT_DEV_BRIEF_TOKENS", "5");
        assert_eq!(brief_budget(), DEFAULT_BRIEF_TOKENS, "too small is ignored");
        std::env::set_var("KWANT_DEV_BRIEF_TOKENS", "20000");
        assert_eq!(brief_budget(), 20_000);
        std::env::remove_var("KWANT_DEV_BRIEF_TOKENS");
    }
}
