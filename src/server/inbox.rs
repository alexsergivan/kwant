//! 0.9.0 V1 Review inbox: `GET /api/inbox` joins what `GET /api/away` lists (questions,
//! approvals, delegations, routine runs) with the tasks waiting in review, in one sorted queue
//! with Mira's groups (release-090/designs/v1-review-inbox.md, ENG) and Ada's D1-D5.
//!
//! The server decides what is safe to approve: `one_tap`, `batchable` and `rollback_ok` are
//! computed here, never in the client. Nothing held is ever one tap or batchable, a `needs_you`
//! never batches, and a verdict of `none` gets neither.
//!
//! Until Juno's verifier (V2/V3, branch juno-090-verifier) lands, review items use Mira's
//! fallbacks: verdict `none` with "{passed} of {total} required checks passed.", round 1 of
//! [`MAX_ROUNDS`] (no Round chip), acceptance lines as `not_checked`, `couldnt_verify` empty, and
//! the last send-back's note with the bot's summary as `previous_round` (no flips, no diff).
//! [`verdict_of`] already maps Juno's serialized `TaskVerdict` (D1) for when it does.
//!
//! `POST /api/away/seen` only moves the marker for finished and failed items: review items stay
//! listed until they are approved, sent back or rolled back.

use super::away::{self, item, last_seen, rfc3339, waiting_approvals, DEFAULT_LOOKBACK_DAYS, MAX_ITEMS, STUCK_AFTER_MINUTES};
use super::spaces::Scope;
use super::AppState;
use crate::db::checkpoints::Checkpoint;
use crate::db::dev_mode::DevCardRow;
use crate::db::dev_prepush::{BLOCKED, NEEDS_YOU_TRIES, PREPUSH_FAILED};
use crate::db::missions::{Evidence, OpenCard, OutcomeCheck, Task, TaskEvent};
use crate::db::spaces::SpaceId;
use crate::db::verifier::TaskVerdict;
use crate::db::DbError;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use std::cmp::Ordering;

/// Rounds a task gets before it comes back to the person whatever the verdict (Juno §4).
pub const MAX_ROUNDS: i64 = 2;
/// Send-backs after which the item asks how to go on (Mira §6) and waits on the person.
pub const SEND_BACKS_TO_ASK: i64 = 3;
/// The Low-risk card appears only when this many items qualify (Mira §5).
pub const BATCH_MIN: usize = 2;
/// Most review tasks read per call.
const MAX_REVIEW: i64 = 100;
/// Lines of a check's output shown as its tail.
const TAIL_LINES: usize = 12;
const TAIL_CHARS: usize = 1200;
const ARGS_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    WaitingOnYou = 1,
    FailedOrStuck = 2,
    ToReview = 3,
    Finished = 4,
}

impl Group {
    pub fn key(self) -> &'static str {
        match self {
            Group::WaitingOnYou => "waiting_on_you",
            Group::FailedOrStuck => "failed_or_stuck",
            Group::ToReview => "to_review",
            Group::Finished => "finished",
        }
    }
    fn of_key(key: &str) -> Group {
        match key {
            "waiting_on_you" => Group::WaitingOnYou,
            "failed_or_stuck" => Group::FailedOrStuck,
            "to_review" => Group::ToReview,
            _ => Group::Finished,
        }
    }
}

/// What the rules of Mira §5 and the grouping need to know about one review item.
#[derive(Debug, Clone, Default)]
pub struct ReviewFacts {
    /// `pass`, `fail`, `needs_you` or `none`.
    pub result: String,
    pub confidence: Option<String>,
    pub risk: Option<String>,
    pub round_n: i64,
    pub round_max: i64,
    /// Times the person sent it back.
    pub send_backs: i64,
    /// `stale` or `approve_error` is set.
    pub approve_failed: bool,
    pub couldnt_verify: usize,
    /// Any held action still waits for Deny / Allow once.
    pub held_pending: bool,
    /// Every required check that counts passed (`checks_summary.status == passed`).
    pub required_passed: bool,
}

/// Mira's grouping: needs_you, a fail in the last round and the 3rd send-back wait on the person;
/// a failed approve is failed or stuck; pass or none is to review; a fail before the last round
/// isn't listed (the bot is on its next round).
pub fn group_of(f: &ReviewFacts) -> Option<Group> {
    if f.result == "needs_you" || f.send_backs >= SEND_BACKS_TO_ASK {
        return Some(Group::WaitingOnYou);
    }
    if f.result == "fail" && f.round_n >= f.round_max {
        return Some(Group::WaitingOnYou);
    }
    if f.approve_failed {
        return Some(Group::FailedOrStuck);
    }
    match f.result.as_str() {
        "fail" => None,
        _ => Some(Group::ToReview),
    }
}

fn high_confidence_pass(f: &ReviewFacts) -> bool {
    f.result == "pass" && f.confidence.as_deref() == Some("high") && f.couldnt_verify == 0 && !f.held_pending
}

/// Inline Approve: Pass, high confidence, nothing it couldn't check, nothing held, risk not high.
pub fn one_tap(f: &ReviewFacts) -> bool {
    high_confidence_pass(f) && f.risk.as_deref() != Some("high")
}

/// Low-risk card: [`one_tap`]'s rules with risk low and every required check passed. A
/// `needs_you` never qualifies (D4): its result isn't `pass`.
pub fn batchable(f: &ReviewFacts) -> bool {
    high_confidence_pass(f) && f.risk.as_deref() == Some("low") && f.required_passed
}

/// Roll back: a before_run checkpoint to go back to, and the work hasn't landed through the Dev
/// desk (a merged PR can't be rolled back from here in 0.9).
pub fn rollback_ok(checkpoints: &[Checkpoint], landed: bool) -> bool {
    !landed && checkpoints.iter().any(|c| c.kind == "before_run" && !c.failed())
}

/// Within to_review: high 0, medium or none 1, low 2; 0 in the other groups.
pub fn risk_rank(group: Group, risk: Option<&str>) -> i64 {
    match (group, risk) {
        (Group::ToReview, Some("high")) => 0,
        (Group::ToReview, Some("low")) => 2,
        (Group::ToReview, _) => 1,
        _ => 0,
    }
}

/// An unread finished item sorts before every read one; `10^13` ms is past the year 2286.
const READ_OFFSET: i64 = 10_000_000_000_000;

/// Groups 1-3: oldest first (`since_ms`). Finished: not read first, then newest, folded into one
/// number so the key stays `[group_rank, risk_rank, age_key, kwant_name, id]` like the fixtures.
pub fn age_key(group: Group, at_ms: i64, unread: bool) -> i64 {
    match group {
        Group::Finished => (if unread { 0 } else { READ_OFFSET }) - at_ms,
        _ => at_ms,
    }
}

pub fn sort_key(group: Group, risk: Option<&str>, at_ms: i64, unread: bool, kwant_name: &str, id: &str) -> Value {
    json!([group as i64, risk_rank(group, risk), age_key(group, at_ms, unread), kwant_name, id])
}

fn cmp_keys(a: &Value, b: &Value) -> Ordering {
    let (Some(a), Some(b)) = (a.as_array(), b.as_array()) else {
        return Ordering::Equal;
    };
    for (x, y) in a.iter().zip(b.iter()) {
        let o = match (x, y) {
            (Value::Number(x), Value::Number(y)) => x.as_i64().unwrap_or(0).cmp(&y.as_i64().unwrap_or(0)),
            (Value::String(x), Value::String(y)) => x.to_lowercase().cmp(&y.to_lowercase()).then_with(|| x.cmp(y)),
            _ => Ordering::Equal,
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

/// Sorts `items` by their `sort_key`, then takes the batchable ones out as the Low-risk card when
/// at least [`BATCH_MIN`] qualify. Returns (items, batch items).
pub fn sort_and_batch(mut items: Vec<Value>) -> (Vec<Value>, Vec<Value>) {
    items.sort_by(|a, b| cmp_keys(&a["sort_key"], &b["sort_key"]));
    let qualifying = items.iter().filter(|i| i["batchable"] == json!(true)).count();
    if qualifying < BATCH_MIN {
        return (items, Vec::new());
    }
    items.into_iter().partition(|i| i["batchable"] != json!(true))
}

/// The one Inbox badge: today's `unseen.total`, the pre-push cards that need the person, every
/// open review item listed (groups 1-3, outside the card) and 1 for the Low-risk card.
pub fn badge(unseen_total: i64, prepush_needs_you: i64, review_listed: i64, has_batch: bool) -> i64 {
    unseen_total + prepush_needs_you + review_listed + i64::from(has_batch)
}

// ---- D1: Juno's TaskVerdict onto Mira's verdict ----

fn engine_of(model: &str) -> String {
    let m = model.to_ascii_lowercase();
    let e = if m.contains("claude") || m.contains("opus") || m.contains("sonnet") || m.contains("haiku") || m.contains("fable") {
        "Claude"
    } else if m.contains("gpt") || m.contains("codex") || m.starts_with("o3") || m.starts_with("o4") {
        "Codex"
    } else if m.contains("grok") {
        "Grok"
    } else if m.contains("gemini") {
        "Gemini"
    } else {
        model
    };
    e.to_string()
}

/// Mira's `verdict` from Juno's latest `TaskVerdict` as JSON (D1), else `none` with the checks
/// line. `steps_run` counts steps not `skipped`.
pub fn verdict_of(latest: Option<&Value>, checks_line: &str) -> Value {
    let Some(v) = latest.filter(|v| v["verdict"].is_string()) else {
        return json!({
            "id": null, "result": "none", "confidence": null, "line": checks_line,
            "checker": null, "checked_at": null, "steps_run": null, "steps_total": null,
        });
    };
    let steps = v["steps"].as_array();
    let model = v["model"].as_str().unwrap_or("");
    let checker = (!model.is_empty()).then(|| {
        json!({
            "engine": engine_of(model),
            "model": model,
            "tier_note": if v["difference"] == json!("tier") { json!("different_tier") } else { Value::Null },
        })
    });
    let line = v["summary"].as_str().filter(|s| !s.trim().is_empty()).map(away::summarize).unwrap_or_else(|| checks_line.to_string());
    json!({
        "id": v["id"],
        "result": v["verdict"],
        "confidence": v["confidence"],
        "line": line,
        "checker": checker,
        "checked_at": v["created_at"],
        "steps_run": steps.map(|s| s.iter().filter(|x| x["status"] != json!("skipped")).count()),
        "steps_total": steps.map(Vec::len),
    })
}

// ---- Review items ----

/// What a review item is built from; read by [`gather`], turned into JSON by [`review_item`].
pub struct ReviewInput {
    pub task: Task,
    pub kwant_name: String,
    pub project: Option<String>,
    pub in_review_since: DateTime<Utc>,
    pub checks: Vec<OutcomeCheck>,
    pub evidence: Vec<Evidence>,
    pub checkpoints: Vec<Checkpoint>,
    pub events: Vec<TaskEvent>,
    /// The owner's open approvals in the task's chat.
    pub held: Vec<(OpenCard, Option<String>)>,
    /// Dev desk review card for the task's PR, and whether a landed card exists for it.
    pub dev_review: Option<DevCardRow>,
    pub landed: bool,
    /// Juno's latest verdict of this review round as JSON ([`current_verdict`]).
    pub latest: Option<Value>,
}

fn acceptance_lines(acceptance: &str) -> Vec<String> {
    acceptance
        .lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '•', ' ']).trim())
        .map(|l| {
            // "1. text" / "1) text"
            let digits = l.chars().take_while(char::is_ascii_digit).count();
            match (digits, l[digits..].chars().next()) {
                (1..=3, Some('.' | ')')) => l[digits + 1..].trim(),
                _ => l,
            }
        })
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// A person's send-backs: review → ready by the person, oldest first.
fn send_backs(events: &[TaskEvent]) -> Vec<&TaskEvent> {
    events
        .iter()
        .filter(|e| e.actor == "person" && e.from_state.as_deref() == Some("review") && e.to_state.as_deref() == Some("ready"))
        .collect()
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    let joined = lines[start..].iter().map(|l| away::mask_words(l)).collect::<Vec<_>>().join("\n");
    let n = joined.chars().count();
    match n > TAIL_CHARS {
        true => format!("…{}", joined.chars().skip(n - TAIL_CHARS + 1).collect::<String>()),
        false => joined,
    }
}

fn check_json(task: &Task, c: &OutcomeCheck) -> Option<Value> {
    let result = match c.status.as_str() {
        "passed" => "pass",
        "failed" | "error" => "fail",
        _ => return None,
    };
    let spec = c.spec_value();
    let command = ["command", "url", "path", "note"].iter().find_map(|k| spec[*k].as_str()).unwrap_or("");
    let owner = task.owner_kwant_id.as_deref().unwrap_or("");
    let ran_by = match c.checked_by.as_deref() {
        Some(by) if !by.is_empty() && by != owner => "checker",
        _ => "author",
    };
    let excerpt = c.output_excerpt.as_deref().unwrap_or("");
    let summary = match excerpt.trim().is_empty() {
        true => format!("{} check {}", c.kind, c.status),
        false => away::summarize(excerpt),
    };
    Some(json!({
        "name": c.kind,
        "command": away::mask_words(command),
        "result": result,
        "summary": summary,
        "ran_by": ran_by,
        "output_tail": tail(excerpt),
    }))
}

/// A GitHub-style PR link: (repo, number).
fn pr_of_url(url: &str) -> Option<(String, u64)> {
    let rest = url.strip_prefix("https://")?;
    let parts: Vec<&str> = rest.split('/').collect();
    match parts.as_slice() {
        [_host, owner, name, "pull" | "pulls" | "merge_requests", n, ..] => Some((format!("{owner}/{name}"), n.parse().ok()?)),
        [_host, owner, name, "-", "merge_requests", n, ..] => Some((format!("{owner}/{name}"), n.parse().ok()?)),
        _ => None,
    }
}

/// The task's PR, from its url evidence; the newest one wins.
pub fn task_pr(evidence: &[Evidence]) -> Option<(String, u64, String)> {
    evidence.iter().rev().filter(|e| e.kind == "url").find_map(|e| {
        let url = e.uri.as_deref()?;
        let (repo, n) = pr_of_url(url)?;
        Some((repo, n, url.to_string()))
    })
}

fn pr_json(pr: Option<&(String, u64, String)>, card: Option<&DevCardRow>) -> Value {
    let facts: Value = card.and_then(|c| serde_json::from_str(&c.facts).ok()).unwrap_or(Value::Null);
    let Some((repo, number, url)) = pr else {
        return Value::Null;
    };
    let files = facts["files"].as_array();
    let sum = |k: &str| files.map(|f| f.iter().filter_map(|x| x[k].as_u64()).sum::<u64>()).unwrap_or(0);
    json!({
        "repo": repo,
        "number": number,
        "url": facts["links"]["pr"].as_str().unwrap_or(url),
        "add": sum("additions"),
        "del": sum("deletions"),
        "files": files.map(Vec::len).unwrap_or(0),
        "ci": facts["ci"].as_str().unwrap_or("none"),
    })
}

fn gate_of(details: &Value, tool_name: &str) -> &'static str {
    if details.get("payment_reason").is_some() {
        return "payment";
    }
    let text = format!("{tool_name} {}", details["arguments"]).to_ascii_lowercase();
    match text.contains("merge") {
        true => "merge",
        false => "approval",
    }
}

fn clip(text: &str, max: usize) -> String {
    match text.chars().count() > max {
        true => format!("{}…", text.chars().take(max - 1).collect::<String>()),
        false => text.to_string(),
    }
}

/// One held action: an approval the task's bot waits on, with the approval card's redaction.
fn held_json(card: &OpenCard, parameters: Option<&str>) -> Value {
    let params = parameters.unwrap_or("");
    let details = away::masked_details(serde_json::from_str(params).unwrap_or(Value::Null));
    let tool = details["tool"].as_str().unwrap_or(&card.text).to_string();
    let args = serde_json::to_string_pretty(&details["arguments"]).unwrap_or_default();
    json!({
        "approval_id": card.id,
        "tool_name": card.text,
        "summary": away::approval_summary(params),
        "args_display": clip(&format!("{tool} {args}"), ARGS_CHARS),
        "gate": gate_of(&details, &card.text),
        "status": "pending",
        "resolved_at": null,
        "result_line": null,
    })
}

/// D7: the verifier's hold (v2-v3-contract §3.4) on a task in review: a required `peer_review`
/// check with `requested_by = "verifier"`, `status = pending` and `spec.verifier = true`, made at
/// or after the task's last move to review. Read from the checks table, so it is the same with or
/// without the verifier's code (#502).
pub fn verifier_hold(checks: &[OutcomeCheck], in_review_since: DateTime<Utc>) -> Option<&OutcomeCheck> {
    checks
        .iter()
        .filter(|c| {
            c.kind == "peer_review"
                && c.required
                && c.requested_by == "verifier"
                && c.status == "pending"
                && c.spec_value()["verifier"] == json!(true)
                && c.created_at >= in_review_since
        })
        .max_by_key(|c| c.created_at)
}

/// D7 "Being checked": the task is in review, the verifier's hold is pending, and no verdict has
/// come in for it (none at all, or only one from before the hold was made: an earlier round's).
pub fn being_checked(task_state: &str, checks: &[OutcomeCheck], in_review_since: DateTime<Utc>, latest: Option<&Value>) -> bool {
    if task_state != "review" {
        return false;
    }
    let Some(hold) = verifier_hold(checks, in_review_since) else { return false };
    let verdict_at = latest
        .filter(|v| v["verdict"].is_string())
        .map(|v| v["created_at"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc)));
    match verdict_at {
        None => true,
        // A verdict we can't date is taken as this round's
        Some(None) => false,
        Some(Some(at)) => at < hold.created_at,
    }
}

/// The task's newest verdict as the JSON [`verdict_of`] and [`review_item`] read (`verdict`,
/// `confidence`, `risk_tier`, `risk_reasons`, `round`, `steps`, `couldnt_verify`, `difference`,
/// `summary`, `created_at`, `id`, `model`), when it belongs to this review round: made since the
/// task's last move to review and not overturned. An earlier round's pass must not offer Approve.
pub fn current_verdict(v: &TaskVerdict, in_review_since: DateTime<Utc>) -> Option<Value> {
    if v.created_at < in_review_since || v.overturned_at.is_some() {
        return None;
    }
    serde_json::to_value(v).ok()
}

/// A watcher's open item (`watch::waiting_items`) in Waiting on you, keyed like the away items.
fn watch_item(mut v: Value, last_seen: Option<DateTime<Utc>>) -> Value {
    let at = v["timestamp"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc));
    let at_ms = at.map(|t| t.timestamp_millis()).unwrap_or(0);
    let unread = match (last_seen, at) {
        (Some(s), Some(a)) => a > s,
        _ => true,
    };
    let name = v["kwant_name"].as_str().unwrap_or("").to_string();
    let id = v["id"].as_str().unwrap_or("").to_string();
    v["group"] = json!(Group::WaitingOnYou.key());
    v["sort_key"] = sort_key(Group::WaitingOnYou, None, at_ms, unread, &name, &id);
    v
}

/// A review item, and the facts its group and buttons came from; `None` when it isn't listed.
pub fn review_item(r: &ReviewInput, last_seen: Option<DateTime<Utc>>) -> Option<(Value, ReviewFacts)> {
    let t = &r.task;
    let summary = super::missions::checks_summary(t, &r.checks);
    let required: Vec<&OutcomeCheck> = r.checks.iter().filter(|c| c.required && super::missions::counts(t, c)).collect();
    let passed = required.iter().filter(|c| c.status == "passed").count();
    let checks_line = match required.len() {
        0 => "No required checks yet.".to_string(),
        n => format!("{passed} of {n} required checks passed."),
    };
    let verdict = verdict_of(r.latest.as_ref(), &checks_line);
    let latest = r.latest.as_ref();

    // Risk: Juno's risk_tier, else the Dev desk card's
    let card: Value = r.dev_review.as_ref().and_then(|c| c.card.as_deref()).and_then(|c| serde_json::from_str(c).ok()).unwrap_or(Value::Null);
    let (risk, reasons) = match latest.and_then(|v| v["risk_tier"].as_str()) {
        Some(tier) => (Some(tier.to_string()), latest.map(|v| v["risk_reasons"].clone()).unwrap_or(Value::Null)),
        None => (card["risk"]["level"].as_str().map(String::from), card["risk"]["reasons"].clone()),
    };
    let reasons: Vec<String> = reasons.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    let risk = risk.filter(|r| matches!(r.as_str(), "low" | "medium" | "high"));

    let backs = send_backs(&r.events);
    let round_n = latest.and_then(|v| v["round"].as_i64()).unwrap_or(1).max(1);
    let couldnt: Vec<Value> = latest
        .and_then(|v| v["couldnt_verify"].as_array())
        .map(|a| a.iter().filter(|x| x["what"].is_string()).map(|x| json!({"what": x["what"], "why": x["why"].as_str().unwrap_or("")})).collect())
        .unwrap_or_default();
    let held: Vec<Value> = r.held.iter().map(|(c, p)| held_json(c, p.as_deref())).collect();

    let facts = ReviewFacts {
        result: verdict["result"].as_str().unwrap_or("none").to_string(),
        confidence: verdict["confidence"].as_str().map(String::from),
        risk: risk.clone(),
        round_n,
        round_max: MAX_ROUNDS,
        send_backs: backs.len() as i64,
        approve_failed: false,
        couldnt_verify: couldnt.len(),
        held_pending: !held.is_empty(),
        required_passed: summary["status"] == json!("passed"),
    };
    let group = group_of(&facts)?;

    let criteria: Vec<Value> = acceptance_lines(&t.acceptance)
        .into_iter()
        .enumerate()
        .map(|(i, text)| json!({"index": i + 1, "text": text, "status": "not_checked", "source": "task", "evidence": null, "note": null, "round_added": 1}))
        .collect();
    let count = |s: &str| criteria.iter().filter(|c| c["status"] == json!(s)).count();
    let criteria_summary = json!({
        "met": count("met") + count("claim_only"),
        "total": criteria.len(),
        "not_met": count("not_met"),
        "not_checked": count("not_checked"),
    });

    // Back from a send-back: the person's note and what the bot said when it came back
    let previous_round = backs.last().filter(|b| b.created_at <= r.in_review_since).map(|b| {
        let note = b.detail_value()["reason"].as_str().map(away::mask_words).unwrap_or_default();
        let bot = r
            .events
            .iter()
            .rev()
            .find(|e| e.to_state.as_deref() == Some("review") && e.created_at >= b.created_at)
            .and_then(|e| e.detail_value()["reason"].as_str().map(away::summarize))
            .unwrap_or_default();
        json!({"sent_back_at": rfc3339(b.created_at), "note": note, "flips": [], "diff": {"files": 0, "add": 0, "del": 0}, "bot_summary": bot})
    });

    let pr = task_pr(&r.evidence);
    let screenshots: Vec<Value> = r
        .evidence
        .iter()
        .filter(|e| e.kind == "screenshot")
        .filter_map(|e| e.uri.as_deref().filter(|u| u.starts_with("https://") || u.starts_with("http://")).map(|u| json!({"url": u, "caption": e.label})))
        .collect();

    let at_ms = r.in_review_since.timestamp_millis();
    let link = json!({"thread_id": t.thread_id, "channel_id": t.channel_id, "direct_kwant_id": t.direct_kwant_id});
    let unread = last_seen.is_none_or(|s| r.in_review_since > s);
    let item = json!({
        "kind": "review",
        "id": t.id,
        "kwant_id": t.owner_kwant_id.clone().unwrap_or_default(),
        "kwant_name": r.kwant_name,
        "title": t.title,
        "summary": verdict["line"],
        "status": t.state,
        "timestamp": rfc3339(r.in_review_since),
        "link": link,
        "group": group.key(),
        "sort_key": sort_key(group, risk.as_deref(), at_ms, unread, &r.kwant_name, &t.id),
        "task_id": t.id,
        "mission_id": t.mission_id,
        "project": r.project,
        "in_review_since": rfc3339(r.in_review_since),
        "round": {"n": round_n, "max": MAX_ROUNDS},
        "send_backs": backs.len(),
        "verdict": verdict,
        "being_checked": being_checked(&t.state, &r.checks, r.in_review_since, latest),
        "risk": {"level": risk, "reasons": reasons},
        "criteria": criteria,
        "criteria_summary": criteria_summary,
        "couldnt_verify": couldnt,
        "decisions": {"total": 0, "top": []},
        "evidence": {
            "checks": r.checks.iter().filter_map(|c| check_json(t, c)).collect::<Vec<_>>(),
            "screenshots": screenshots,
            "pr": pr_json(pr.as_ref(), r.dev_review.as_ref()),
        },
        "held_actions": held,
        "previous_round": previous_round,
        "stale": null,
        "approve_error": null,
        "one_tap": one_tap(&facts),
        "batchable": batchable(&facts),
        "rollback_ok": rollback_ok(&r.checkpoints, r.landed),
    });
    Some((item, facts))
}

/// An away item in its inbox group: waiting, failed (or stuck) and finished.
fn away_item(row: crate::db::away::AwayRow, group: Group, last_seen: Option<DateTime<Utc>>) -> Value {
    let at = row.at;
    let unread = last_seen.is_none_or(|s| at > s);
    let mut v = item(row);
    let name = v["kwant_name"].as_str().unwrap_or("").to_string();
    let id = v["id"].as_str().unwrap_or("").to_string();
    v["group"] = json!(group.key());
    v["sort_key"] = sort_key(group, None, at.timestamp_millis(), unread, &name, &id);
    v
}

// ---- Reading ----

/// The owner's approvals that wait in the task's chat: its thread, else its channel, else its
/// direct chat. With their stored parameters, for the summary.
async fn held_for(state: &AppState, task: &Task) -> Result<Vec<(OpenCard, Option<String>)>, DbError> {
    let Some(owner) = task.owner_kwant_id.as_deref() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for card in state.db.open_cards_of(owner).await? {
        if card.kind != "approval" {
            continue;
        }
        let same = match (&task.thread_id, &task.channel_id, &task.direct_kwant_id) {
            (Some(t), _, _) => card.thread_id.as_ref() == Some(t),
            (None, Some(c), _) => card.thread_id.is_none() && card.channel_id.as_ref() == Some(c),
            (None, None, Some(d)) => card.thread_id.is_none() && card.channel_id.is_none() && card.direct_kwant_id.as_ref() == Some(d),
            _ => false,
        };
        if same {
            let params = state.db.get_approval(&card.id).await?.map(|a| a.parameters);
            out.push((card, params));
        }
    }
    Ok(out)
}

fn same_pr(row: &DevCardRow, repo: &str, number: u64, url: &str) -> bool {
    let facts: Value = serde_json::from_str(&row.facts).unwrap_or(Value::Null);
    facts["links"]["pr"].as_str() == Some(url) || (facts["repo"].as_str().is_some_and(|r| r.eq_ignore_ascii_case(repo)) && facts["pr"].as_u64() == Some(number))
}

async fn gather(state: &AppState, space: &SpaceId) -> Result<Vec<ReviewInput>, DbError> {
    let tasks = state.db.list_tasks(space, Some("review"), None, None, MAX_REVIEW).await?;
    if tasks.is_empty() {
        return Ok(Vec::new());
    }
    let reviews = state.db.dev_cards(space.as_str(), "review", None, 100).await?;
    let landed = state.db.dev_cards(space.as_str(), "landed", None, 100).await?;
    let ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
    let mut verdicts = state.db.latest_verdicts(&ids).await?;
    let mut out = Vec::with_capacity(tasks.len());
    for task in tasks {
        let events = state.db.task_events(&task.id, 500).await?;
        let since = events.iter().rev().find(|e| e.to_state.as_deref() == Some("review")).map(|e| e.created_at).unwrap_or(task.updated_at);
        let kwant_name = match task.owner_kwant_id.as_deref() {
            Some(k) => state.db.get_kwant(k).await?.map(|k| k.name),
            None => None,
        }
        .unwrap_or_else(|| "A bot".into());
        let project = match task.mission_id.as_deref() {
            Some(m) => state.db.get_mission(m).await?.map(|m| m.title),
            None => None,
        };
        let evidence = state.db.list_evidence(&task.id).await?;
        let pr = task_pr(&evidence);
        let dev_review = pr.as_ref().and_then(|(repo, n, url)| reviews.iter().find(|c| same_pr(c, repo, *n, url)).cloned());
        let is_landed = pr.as_ref().is_some_and(|(repo, n, url)| landed.iter().any(|c| same_pr(c, repo, *n, url)));
        out.push(ReviewInput {
            checks: state.db.list_checks(&task.id).await?,
            checkpoints: state.db.checkpoints_of(&task.id).await?,
            held: held_for(state, &task).await?,
            kwant_name,
            project,
            in_review_since: since,
            evidence,
            events,
            dev_review,
            landed: is_landed,
            latest: verdicts.remove(&task.id).and_then(|v| current_verdict(&v, since)),
            task,
        });
    }
    Ok(out)
}

#[derive(Debug, serde::Deserialize)]
pub struct InboxQuery {
    pub since: Option<String>,
}

/// `GET /api/inbox?since=<RFC 3339>&space=`: `since` defaults to the last-seen marker, else 7
/// days ago, as on `GET /api/away`; it only bounds finished and failed items.
pub async fn get(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<InboxQuery>) -> Response {
    let now = Utc::now();
    let Ok(seen) = last_seen(&state, &space).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let default_since = seen.unwrap_or(now - Duration::days(DEFAULT_LOOKBACK_DAYS));
    let since = match q.since.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => default_since,
        Some(s) => match DateTime::parse_from_rfc3339(s) {
            Ok(t) => t.with_timezone(&Utc),
            Err(_) => {
                let msg = "since must be an RFC 3339 time, e.g. 2026-10-17T09:00:00Z";
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response();
            }
        },
    };
    let stuck_before = now - Duration::minutes(STUCK_AFTER_MINUTES);

    let result = async {
        let mut waiting = state.db.away_open_questions(&space, MAX_ITEMS).await?;
        let approvals = waiting_approvals(&state, &space).await?;
        let waiting_count = state.db.away_count_open_questions(&space).await? + approvals.len() as i64;
        waiting.extend(approvals);
        let finished = state.db.away_finished(&space, since, MAX_ITEMS).await?;
        let failed = state.db.away_failed(&space, since, stuck_before, MAX_ITEMS).await?;
        let (unseen_finished, unseen_failed) = state.db.away_counts(&space, default_since, stuck_before).await?;
        let prepush = state.db.prepush_cards(space.as_str(), 50).await?;
        let prepush_needs_you = prepush.iter().filter(|c| (c.status == BLOCKED || c.status == PREPUSH_FAILED) && c.tries >= NEEDS_YOU_TRIES).count() as i64;
        let reviews = gather(&state, &space).await?;
        Ok::<_, DbError>((waiting, finished, failed, waiting_count + unseen_finished + unseen_failed, prepush_needs_you, reviews))
    }
    .await;
    let Ok((waiting, finished, failed, unseen_total, prepush_needs_you, reviews)) = result else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    // What a mission watcher left for the person (P1), listed and counted as on /api/away
    let watch = super::missions::watch::waiting_items(&state, &space).await;
    let unseen_total = unseen_total + watch.len() as i64;

    let mut items: Vec<Value> = Vec::new();
    items.extend(waiting.into_iter().map(|r| away_item(r, Group::WaitingOnYou, seen)));
    items.extend(watch.into_iter().map(|v| watch_item(v, seen)));
    items.extend(failed.into_iter().map(|r| away_item(r, Group::FailedOrStuck, seen)));
    items.extend(finished.into_iter().map(|r| away_item(r, Group::Finished, seen)));
    items.extend(reviews.iter().filter_map(|r| review_item(r, seen)).map(|(v, _)| v));
    Json(response(items, unseen_total, prepush_needs_you, since, seen, now)).into_response()
}

/// The response from the unsorted items: sorted, the Low-risk card taken out, counts and badge.
pub fn response(items: Vec<Value>, unseen_total: i64, prepush_needs_you: i64, since: DateTime<Utc>, seen: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Value {
    let (items, batch) = sort_and_batch(items);
    let in_group = |g: Group| items.iter().filter(|i| Group::of_key(i["group"].as_str().unwrap_or("")) == g).count();
    let review_listed = items.iter().filter(|i| i["kind"] == json!("review")).count() as i64;
    let counts = json!({
        "waiting_on_you": in_group(Group::WaitingOnYou),
        "failed_or_stuck": in_group(Group::FailedOrStuck),
        "to_review": in_group(Group::ToReview) + batch.len(),
        "finished": in_group(Group::Finished),
        "badge": badge(unseen_total, prepush_needs_you, review_listed, !batch.is_empty()),
    });
    let batch_json = match batch.is_empty() {
        true => Value::Null,
        false => json!({"ids": batch.iter().map(|i| i["id"].clone()).collect::<Vec<_>>(), "count": batch.len()}),
    };
    json!({
        "now": rfc3339(now),
        "since": rfc3339(since),
        "last_seen": seen.map(rfc3339),
        "counts": counts,
        "items": items,
        "batch": batch_json,
        "batch_items": batch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(result: &str) -> ReviewFacts {
        ReviewFacts {
            result: result.into(),
            confidence: Some("high".into()),
            risk: Some("low".into()),
            round_n: 1,
            round_max: MAX_ROUNDS,
            required_passed: true,
            ..Default::default()
        }
    }

    #[test]
    fn grouping_follows_mira() {
        assert_eq!(group_of(&facts("needs_you")), Some(Group::WaitingOnYou));
        assert_eq!(group_of(&facts("pass")), Some(Group::ToReview));
        assert_eq!(group_of(&facts("none")), Some(Group::ToReview));
        // A fail before the round limit isn't listed: the bot is on its next round
        assert_eq!(group_of(&facts("fail")), None);
        assert_eq!(group_of(&ReviewFacts { round_n: 2, ..facts("fail") }), Some(Group::WaitingOnYou));
        assert_eq!(group_of(&ReviewFacts { send_backs: 3, ..facts("pass") }), Some(Group::WaitingOnYou));
        assert_eq!(group_of(&ReviewFacts { send_backs: 2, ..facts("pass") }), Some(Group::ToReview));
        assert_eq!(group_of(&ReviewFacts { approve_failed: true, ..facts("pass") }), Some(Group::FailedOrStuck));
        // needs_you wins over a failed approve
        assert_eq!(group_of(&ReviewFacts { approve_failed: true, ..facts("needs_you") }), Some(Group::WaitingOnYou));
    }

    #[test]
    fn nothing_held_or_unverified_is_one_tap_or_batchable() {
        let ok = facts("pass");
        assert!(one_tap(&ok) && batchable(&ok));
        let held = ReviewFacts { held_pending: true, ..ok.clone() };
        assert!(!one_tap(&held) && !batchable(&held));
        let couldnt = ReviewFacts { couldnt_verify: 1, ..ok.clone() };
        assert!(!one_tap(&couldnt) && !batchable(&couldnt));
        for r in ["none", "needs_you", "fail"] {
            assert!(!one_tap(&facts(r)) && !batchable(&facts(r)), "{r}");
        }
        let medium_conf = ReviewFacts { confidence: Some("medium".into()), ..ok.clone() };
        assert!(!one_tap(&medium_conf) && !batchable(&medium_conf));
        // Medium risk: one tap, not batchable; high risk: neither
        let medium = ReviewFacts { risk: Some("medium".into()), ..ok.clone() };
        assert!(one_tap(&medium) && !batchable(&medium));
        let unknown = ReviewFacts { risk: None, ..ok.clone() };
        assert!(one_tap(&unknown) && !batchable(&unknown));
        let high = ReviewFacts { risk: Some("high".into()), ..ok.clone() };
        assert!(!one_tap(&high) && !batchable(&high));
        let checks_open = ReviewFacts { required_passed: false, ..ok };
        assert!(one_tap(&checks_open) && !batchable(&checks_open));
    }

    #[test]
    fn risk_and_age_keys() {
        assert_eq!(risk_rank(Group::ToReview, Some("high")), 0);
        assert_eq!(risk_rank(Group::ToReview, Some("medium")), 1);
        assert_eq!(risk_rank(Group::ToReview, None), 1);
        assert_eq!(risk_rank(Group::ToReview, Some("low")), 2);
        for g in [Group::WaitingOnYou, Group::FailedOrStuck, Group::Finished] {
            assert_eq!(risk_rank(g, Some("low")), 0);
            assert_eq!(risk_rank(g, Some("high")), 0);
        }
        // Groups 1-3: oldest first
        assert!(age_key(Group::ToReview, 1_000, true) < age_key(Group::ToReview, 2_000, true));
        assert_eq!(age_key(Group::WaitingOnYou, 1_700_000_000_000, false), 1_700_000_000_000);
        // Finished: unread first, then newest
        let (old_unread, new_unread) = (age_key(Group::Finished, 1_700_000_000_000, true), age_key(Group::Finished, 1_800_000_000_000, true));
        let (old_read, new_read) = (age_key(Group::Finished, 1_700_000_000_000, false), age_key(Group::Finished, 1_800_000_000_000, false));
        assert!(new_unread < old_unread && old_unread < new_read && new_read < old_read);
        assert!(old_read < 9_007_199_254_740_991, "stays a safe JS integer");
    }

    fn it(group: Group, risk: Option<&str>, at: i64, unread: bool, name: &str, id: &str, batchable: bool) -> Value {
        json!({"id": id, "kind": "review", "group": group.key(), "sort_key": sort_key(group, risk, at, unread, name, id), "batchable": batchable})
    }

    fn ids(items: &[Value]) -> Vec<&str> {
        items.iter().map(|i| i["id"].as_str().unwrap()).collect()
    }

    #[test]
    fn sort_order_across_the_four_groups() {
        let items = vec![
            it(Group::Finished, None, 5_000, false, "Ann", "f-read-new", false),
            it(Group::Finished, None, 1_000, true, "Ann", "f-unread-old", false),
            it(Group::Finished, None, 4_000, true, "Ann", "f-unread-new", false),
            it(Group::ToReview, Some("low"), 1_000, true, "Ann", "r-low", false),
            it(Group::ToReview, None, 3_000, true, "Ann", "r-none-new", false),
            it(Group::ToReview, Some("medium"), 2_000, true, "Ann", "r-medium-old", false),
            it(Group::ToReview, Some("high"), 9_000, true, "Ann", "r-high", false),
            it(Group::FailedOrStuck, None, 7_000, true, "Ann", "x-new", false),
            it(Group::FailedOrStuck, None, 6_000, true, "Ann", "x-old", false),
            it(Group::WaitingOnYou, None, 8_000, true, "zed", "w-zed", false),
            it(Group::WaitingOnYou, None, 8_000, true, "Bea", "w-bea-2", false),
            it(Group::WaitingOnYou, None, 8_000, true, "Bea", "w-bea-1", false),
            it(Group::WaitingOnYou, None, 9_500, true, "Ann", "w-late", false),
        ];
        let (sorted, batch) = sort_and_batch(items);
        assert!(batch.is_empty());
        assert_eq!(
            ids(&sorted),
            [
                "w-bea-1", "w-bea-2", "w-zed", "w-late", // oldest first, then name A→Z (any case), then id
                "x-old", "x-new",
                "r-high", "r-medium-old", "r-none-new", "r-low", // highest risk first, medium = none by age
                "f-unread-new", "f-unread-old", "f-read-new",
            ]
        );
    }

    #[test]
    fn batch_needs_two_and_leaves_the_queue() {
        let one = vec![it(Group::ToReview, Some("low"), 1, true, "A", "a", true), it(Group::ToReview, Some("high"), 2, true, "B", "b", false)];
        let (items, batch) = sort_and_batch(one);
        assert_eq!(ids(&items), ["b", "a"]);
        assert!(batch.is_empty(), "a single qualifying item stays a normal row");
        let two = vec![
            it(Group::ToReview, Some("low"), 1, true, "A", "a", true),
            it(Group::ToReview, Some("high"), 2, true, "B", "b", false),
            it(Group::ToReview, Some("low"), 3, true, "C", "c", true),
        ];
        let now = Utc::now();
        let r = response(two, 4, 1, now, None, now);
        assert_eq!(ids(r["items"].as_array().unwrap()), ["b"]);
        assert_eq!(r["batch"], json!({"ids": ["a", "c"], "count": 2}));
        assert_eq!(ids(r["batch_items"].as_array().unwrap()), ["a", "c"]);
        assert_eq!(r["counts"]["to_review"], 3, "the card's items still count as to review");
        // 4 unseen + 1 pre-push + 1 review row + 1 for the card
        assert_eq!(r["counts"]["badge"], 7);
    }

    #[test]
    fn badge_arithmetic() {
        assert_eq!(badge(0, 0, 0, false), 0);
        assert_eq!(badge(3, 0, 0, false), 3, "today's unseen.total alone");
        assert_eq!(badge(3, 2, 0, false), 5, "plus pre-push cards that need you");
        assert_eq!(badge(3, 2, 4, false), 9, "plus open review items");
        assert_eq!(badge(3, 2, 4, true), 10, "the Low-risk card counts 1");
        let now = Utc::now();
        let items = vec![
            json!({"id": "q1", "kind": "question", "group": "waiting_on_you", "sort_key": sort_key(Group::WaitingOnYou, None, 1, true, "A", "q1")}),
            json!({"id": "f1", "kind": "delegation", "group": "finished", "sort_key": sort_key(Group::Finished, None, 1, true, "A", "f1")}),
            it(Group::WaitingOnYou, None, 2, true, "A", "r1", false),
            it(Group::ToReview, None, 2, true, "A", "r2", false),
        ];
        // Away items are in unseen_total already; only review rows add
        let r = response(items, 2, 0, now, None, now);
        assert_eq!(r["counts"]["badge"], 4);
        assert_eq!(r["counts"]["waiting_on_you"], 2);
        assert_eq!(r["counts"]["finished"], 1);
        assert_eq!(r["batch"], Value::Null);
    }

    fn check(kind: &str, required: bool, by: &str, status: &str, spec: Value, at: DateTime<Utc>) -> OutcomeCheck {
        OutcomeCheck {
            id: format!("c-{kind}-{by}-{status}"),
            task_id: "t".into(),
            kind: kind.into(),
            spec: spec.to_string(),
            required,
            approved: true,
            status: status.into(),
            output_excerpt: None,
            checked_by: None,
            requested_by: by.into(),
            approved_by: Some("kwant".into()),
            evidence_id: None,
            created_at: at,
            started_at: None,
            finished_at: None,
        }
    }

    /// D7: "Being checked" only for a pending verifier hold made in this review, with no verdict
    /// for it yet.
    #[test]
    fn being_checked_needs_this_reviews_pending_hold_and_no_verdict() {
        let since = Utc::now();
        let at = since + Duration::seconds(1);
        let hold = || check("peer_review", true, "verifier", "pending", json!({"verifier": true}), at);
        assert!(being_checked("review", &[hold()], since, None));
        assert!(!being_checked("review", &[], since, None), "no hold");
        assert!(!being_checked("ready", &[hold()], since, None), "not in review");
        assert!(!being_checked("review", &[check("peer_review", true, "verifier", "passed", json!({"verifier": true}), at)], since, None), "settled");
        assert!(!being_checked("review", &[check("peer_review", true, "verifier", "pending", json!({"verifier": true}), since - Duration::seconds(1))], since, None), "from an earlier review");
        assert!(!being_checked("review", &[check("peer_review", true, "k2", "pending", json!({}), at)], since, None), "a bot's peer review");
        assert!(!being_checked("review", &[check("peer_review", true, "verifier", "pending", json!({}), at)], since, None), "no spec.verifier");
        assert!(!being_checked("review", &[check("peer_review", false, "verifier", "pending", json!({"verifier": true}), at)], since, None), "optional");
        assert!(!being_checked("review", &[check("command", true, "verifier", "pending", json!({"verifier": true}), at)], since, None), "not peer_review");
        // Juno's verdicts (#502): an earlier round's doesn't count, this hold's does
        let verdict = |t: DateTime<Utc>| json!({"verdict": "fail", "created_at": t.to_rfc3339()});
        assert!(being_checked("review", &[hold()], since, Some(&verdict(since - Duration::minutes(5)))));
        assert!(!being_checked("review", &[hold()], since, Some(&verdict(at + Duration::seconds(1)))));
        assert!(!being_checked("review", &[hold()], since, Some(&json!({"verdict": "needs_you"}))), "an undated verdict counts");
        assert!(being_checked("review", &[hold()], since, Some(&json!({"verdict": null}))), "not a verdict");
    }

    #[test]
    fn verdict_falls_back_to_none_and_maps_junos_names() {
        let none = verdict_of(None, "2 of 3 required checks passed.");
        assert_eq!(none["result"], "none");
        assert_eq!(none["id"], Value::Null);
        assert_eq!(none["line"], "2 of 3 required checks passed.");
        assert_eq!(none["checker"], Value::Null);
        let juno = json!({
            "id": "tv_1", "verdict": "pass", "confidence": "high", "model": "claude-opus-5-5", "difference": "tier",
            "steps": [{"status": "passed"}, {"status": "skipped"}, {"status": "failed"}],
            "round": 2, "summary": "All three criteria met.", "created_at": "2026-11-10T15:00:00.000Z",
        });
        let v = verdict_of(Some(&juno), "x");
        assert_eq!(v["id"], "tv_1");
        assert_eq!(v["result"], "pass");
        assert_eq!(v["confidence"], "high");
        assert_eq!(v["line"], "All three criteria met.");
        assert_eq!(v["checker"], json!({"engine": "Claude", "model": "claude-opus-5-5", "tier_note": "different_tier"}));
        assert_eq!(v["steps_run"], 2);
        assert_eq!(v["steps_total"], 3);
        assert_eq!(v["checked_at"], "2026-11-10T15:00:00.000Z");
        let same = verdict_of(Some(&json!({"verdict": "fail", "model": "gpt-5", "difference": "engine"})), "x");
        assert_eq!(same["checker"]["tier_note"], Value::Null);
        assert_eq!(same["checker"]["engine"], "Codex");
    }

    fn checkpoint(kind: &str, error: Option<&str>) -> Checkpoint {
        Checkpoint {
            id: format!("cp_{kind}"),
            task_id: "t1".into(),
            mission_id: None,
            kind: kind.into(),
            claim_id: None,
            turn_id: None,
            attempt: 1,
            manifest_path: String::new(),
            manifest_sha256: String::new(),
            folder: String::new(),
            files: 3,
            bytes: 10,
            skipped: 0,
            git_head: None,
            error: error.map(String::from),
            created_by: "kwant".into(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn rollback_needs_a_before_run_checkpoint_and_no_landed_card() {
        assert!(rollback_ok(&[checkpoint("before_run", None)], false));
        assert!(!rollback_ok(&[checkpoint("before_run", None)], true), "a Dev desk landed card never rolls back");
        assert!(!rollback_ok(&[checkpoint("after_run", None), checkpoint("manual", None)], false));
        assert!(!rollback_ok(&[checkpoint("before_run", Some("disk full"))], false), "a failed snapshot can't be restored");
        assert!(!rollback_ok(&[], false));
    }

    #[test]
    fn acceptance_lines_become_criteria_text() {
        assert_eq!(acceptance_lines("- one\n* two\n\n3. three\n10) ten\n  plain  "), ["one", "two", "three", "ten", "plain"]);
        assert!(acceptance_lines("  \n").is_empty());
    }

    #[test]
    fn pr_links_parse() {
        assert_eq!(pr_of_url("https://github.com/oleks/kwant/pull/502"), Some(("oleks/kwant".into(), 502)));
        assert_eq!(pr_of_url("https://github.com/oleks/kwant/pull/502/files"), Some(("oleks/kwant".into(), 502)));
        assert_eq!(pr_of_url("https://gitlab.com/a/b/-/merge_requests/7"), Some(("a/b".into(), 7)));
        assert_eq!(pr_of_url("https://github.com/oleks/kwant/issues/5"), None);
        assert_eq!(pr_of_url("http://github.com/oleks/kwant/pull/5"), None);
    }

    #[test]
    fn held_actions_name_their_gate_and_hide_secrets() {
        let card = OpenCard {
            kind: "approval".into(),
            id: "ap1".into(),
            text: "Bash".into(),
            context: None,
            channel_id: None,
            direct_kwant_id: None,
            thread_id: Some("th1".into()),
            thread_kwant_id: None,
        };
        let p = r#"{"tool":"Bash","prod_floor":"merge","arguments":{"command":"gh pr merge 512","api_token":"sk-ant-api03-abcdefghijklmnop1234"}}"#;
        let h = held_json(&card, Some(p));
        assert_eq!(h["gate"], "merge");
        assert_eq!(h["status"], "pending");
        let shown = h.to_string();
        assert!(!shown.contains("sk-ant"), "{shown}");
        assert!(h["args_display"].as_str().unwrap().starts_with("Bash {"));
        let pay = held_json(&card, Some(r#"{"payment_reason":"checkout","arguments":{}}"#));
        assert_eq!(pay["gate"], "payment");
        assert_eq!(held_json(&card, Some(r#"{"arguments":{"to":"x"}}"#))["gate"], "approval");
    }

    #[test]
    fn a_watch_item_waits_on_you_keyed_like_the_away_items() {
        let at = "2026-11-11T06:27:00Z";
        let v = watch_item(json!({"kind": "watch", "id": "wc-1", "kwant_name": "Rigel", "timestamp": at}), None);
        let ms = DateTime::parse_from_rfc3339(at).unwrap().timestamp_millis();
        assert_eq!(v["group"], "waiting_on_you");
        assert_eq!(v["sort_key"], sort_key(Group::WaitingOnYou, None, ms, true, "Rigel", "wc-1"));
        assert_eq!(v["sort_key"][0], json!(1));
    }
}
