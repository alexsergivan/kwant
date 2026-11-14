//! V3, the task contract (0.9.0 wave 3): what must not change, the criteria with their status and
//! evidence, open issues and assumptions, and risk flags, on a task or a hand-off card. The lead
//! and the person write it; the owner can only add to it and mark criteria with evidence. A `met`
//! without evidence is a claim, never shown as met. Contract:
//! `/home/maus/shared/ship-software-safely/release-090/v2-v3-contract.md` §1, docs §24.

use crate::db::missions::Task;
use crate::server::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const STATUSES: [&str; 3] = ["met", "not_met", "not_checked"];
pub const ISSUE_KINDS: [&str; 2] = ["open_issue", "assumption"];
pub const MAX_ITEMS: usize = 20;
pub const MAX_FLAGS: usize = 8;
pub const MAX_TEXT: usize = 300;
pub const MAX_FLAG_CHARS: usize = 40;
/// Parts of the person's request on one contract.
pub const MAX_PARTS: usize = 12;
/// Criteria derived from the acceptance, one per line.
pub const MAX_FROM_ACCEPTANCE: usize = 8;
pub const TOOL: &str = "task_contract";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Criterion {
    pub id: String,
    pub text: String,
    pub status: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    /// `lead`, `owner`, `person` or `acceptance`.
    pub by: String,
    /// 1 plus the number of verdicts on the task when it was added; set once, by kwant (§9.3).
    /// Criteria stored before this field read as round 1.
    #[serde(default = "round_one")]
    pub round_added: i64,
}

fn round_one() -> i64 {
    1
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub by: String,
}

/// One part of the person's request, in their own words, and the criteria that prove it
/// (request-parts, roadmap Next 12a). The parts together cover every word of the request; a part
/// with no criterion linked has no check.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub id: String,
    pub text: String,
    /// Ids of this contract's criteria.
    #[serde(default)]
    pub criteria: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Contract {
    #[serde(default)]
    pub must_not_change: Vec<String>,
    #[serde(default)]
    pub criteria: Vec<Criterion>,
    #[serde(default)]
    pub open_issues: Vec<Issue>,
    #[serde(default)]
    pub risk_flags: Vec<String>,
    /// Written by the lead or the person; stored before this field, a contract has none, and each
    /// criterion reads as its own part.
    #[serde(default)]
    pub parts: Vec<Part>,
}

/// Who writes: the person, the lead (who handed it out or leads its mission), or the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Person,
    Lead,
    Owner,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::Person => "person",
            Role::Lead => "lead",
            Role::Owner => "owner",
        }
    }
}

fn clip(s: &str) -> String {
    s.trim().chars().take(MAX_TEXT).collect()
}

/// One criterion per non-empty line of the acceptance (bullets trimmed), at most
/// [`MAX_FROM_ACCEPTANCE`], all `not_checked`.
pub fn from_acceptance(acceptance: &str) -> Vec<Criterion> {
    acceptance
        .lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '•']).trim())
        .filter(|l| !l.is_empty())
        .take(MAX_FROM_ACCEPTANCE)
        .enumerate()
        .map(|(i, l)| Criterion { id: format!("c{}", i + 1), text: clip(l), status: "not_checked".into(), evidence: Vec::new(), by: "acceptance".into(), round_added: 1 })
        .collect()
}

fn next_id(prefix: char, used: &[&str]) -> String {
    let n = used.iter().filter_map(|id| id.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok())).max().unwrap_or(0);
    format!("{prefix}{}", n + 1)
}

fn strings(v: &Value, what: &str, max: usize, max_chars: usize) -> Result<Option<Vec<String>>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let arr = v.as_array().ok_or(format!("{what} must be a list of strings"))?;
    let mut out = Vec::new();
    for x in arr {
        let s = x.as_str().ok_or(format!("{what} must be a list of strings"))?.trim();
        if s.is_empty() {
            continue;
        }
        if s.chars().count() > max_chars {
            return Err(format!("each {what} entry is at most {max_chars} characters"));
        }
        if !out.iter().any(|o: &String| o == s) {
            out.push(s.to_string());
        }
    }
    if out.len() > max {
        return Err(format!("at most {max} {what} entries"));
    }
    Ok(Some(out))
}

/// Applies a write (the tool's or the route's body) as `role`. `evidence_on_task` are the task's
/// evidence ids. The owner may add, and set status and evidence; anything else it tries is
/// refused with the item it was about, and nothing changes.
pub fn apply(c: Contract, body: &Value, role: Role, evidence_on_task: &[String]) -> Result<(Contract, Vec<&'static str>), String> {
    apply_in_round(c, body, role, evidence_on_task, 1)
}

/// [`apply`] when the task is in verifier round `round` (1 + its verdicts): new criteria get it as
/// `round_added`, and nothing in the body can change it.
pub fn apply_in_round(mut c: Contract, body: &Value, role: Role, evidence_on_task: &[String], round: i64) -> Result<(Contract, Vec<&'static str>), String> {
    let mut fields = Vec::new();
    let owner = role == Role::Owner;
    if let Some(list) = strings(&body["must_not_change"], "must_not_change", MAX_ITEMS, MAX_TEXT)? {
        if owner {
            if let Some(gone) = c.must_not_change.iter().find(|m| !list.contains(m)) {
                return Err(format!("You can't remove or reword \"{gone}\" from must_not_change; only the lead or the person can."));
            }
        }
        c.must_not_change = list;
        fields.push("must_not_change");
    }
    if let Some(list) = strings(&body["risk_flags"], "risk_flags", MAX_FLAGS, MAX_FLAG_CHARS)? {
        let list: Vec<String> = list.into_iter().map(|f| f.to_lowercase()).collect();
        if owner {
            if let Some(gone) = c.risk_flags.iter().find(|f| !list.contains(f)) {
                return Err(format!("You can't remove the risk flag \"{gone}\"; only the lead or the person can."));
            }
        }
        c.risk_flags = list;
        fields.push("risk_flags");
    }
    if !body["criteria"].is_null() {
        let items = body["criteria"].as_array().ok_or("criteria must be a list")?;
        for it in items {
            criterion(&mut c, it, role, evidence_on_task, round)?;
        }
        if c.criteria.len() > MAX_ITEMS {
            return Err(format!("at most {MAX_ITEMS} criteria"));
        }
        fields.push("criteria");
    }
    if !body["open_issues"].is_null() {
        let items = body["open_issues"].as_array().ok_or("open_issues must be a list")?;
        for it in items {
            issue(&mut c, it, role)?;
        }
        if c.open_issues.len() > MAX_ITEMS {
            return Err(format!("at most {MAX_ITEMS} open issues"));
        }
        fields.push("open_issues");
    }
    // A removed criterion is no longer linked to any part
    let ids: Vec<String> = c.criteria.iter().map(|k| k.id.clone()).collect();
    for p in &mut c.parts {
        p.criteria.retain(|id| ids.contains(id));
    }
    if !body["parts"].is_null() {
        let items = body["parts"].as_array().ok_or("parts must be a list")?;
        for it in items {
            part(&mut c, it, role)?;
        }
        if c.parts.len() > MAX_PARTS {
            return Err(format!("at most {MAX_PARTS} parts"));
        }
        fields.push("parts");
    }
    if fields.is_empty() {
        return Err("Nothing to change: give must_not_change, criteria, open_issues, risk_flags or parts.".into());
    }
    Ok((c, fields))
}

fn criterion(c: &mut Contract, it: &Value, role: Role, evidence_on_task: &[String], round: i64) -> Result<(), String> {
    let id = it["id"].as_str().map(str::trim).filter(|s| !s.is_empty());
    let text = it["text"].as_str().map(clip).filter(|s| !s.is_empty());
    let status = it["status"].as_str().map(str::trim);
    if let Some(s) = status.filter(|s| !STATUSES.contains(s)) {
        return Err(format!("status must be met, not_met or not_checked (got {s})"));
    }
    let evidence: Option<Vec<String>> = it.get("evidence").filter(|v| !v.is_null()).map(|v| {
        v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
    });
    if let Some(bad) = evidence.iter().flatten().find(|e| !evidence_on_task.contains(e)) {
        return Err(format!("Evidence {bad} isn't attached to this task."));
    }
    let remove = it["remove"].as_bool() == Some(true);
    match id.and_then(|id| c.criteria.iter().position(|x| x.id == id)) {
        Some(i) => {
            if remove {
                if role == Role::Owner {
                    return Err(format!("You can't remove criterion {}; only the lead or the person can.", c.criteria[i].id));
                }
                c.criteria.remove(i);
                return Ok(());
            }
            let cur = &mut c.criteria[i];
            if let Some(t) = text.filter(|t| *t != cur.text) {
                if role == Role::Owner && cur.by != "owner" {
                    return Err(format!("You can't reword criterion {} (the {} wrote it); set its status and evidence instead.", cur.id, cur.by));
                }
                cur.text = t;
            }
            if let Some(s) = status {
                cur.status = s.to_string();
            }
            if let Some(e) = evidence {
                cur.evidence = e;
            }
        }
        None => {
            if let Some(id) = id.filter(|_| text.is_none()) {
                return Err(format!("No criterion {id}; give text to add a new one."));
            }
            let Some(text) = text else { return Err("A new criterion needs text.".into()) };
            let used: Vec<&str> = c.criteria.iter().map(|x| x.id.as_str()).collect();
            let id = next_id('c', &used);
            c.criteria.push(Criterion { id, text, status: status.unwrap_or("not_checked").into(), evidence: evidence.unwrap_or_default(), by: role.as_str().into(), round_added: round.max(1) });
        }
    }
    Ok(())
}

/// A part write. The lead or the person adds, rewords, relinks or removes a part; the owner can
/// only link more criteria to an existing part (never unlink, reword, add or remove one).
fn part(c: &mut Contract, it: &Value, role: Role) -> Result<(), String> {
    let id = it["id"].as_str().map(str::trim).filter(|s| !s.is_empty());
    let text = it["text"].as_str().map(clip).filter(|s| !s.is_empty());
    let remove = it["remove"].as_bool() == Some(true);
    let links: Option<Vec<String>> = match it.get("criteria").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => {
            let arr = v.as_array().ok_or("a part's criteria must be a list of criterion ids")?;
            let mut out: Vec<String> = Vec::new();
            for x in arr {
                let k = x.as_str().ok_or("a part's criteria must be a list of criterion ids")?.trim();
                if k.is_empty() || out.iter().any(|o| o == k) {
                    continue;
                }
                if !c.criteria.iter().any(|cr| cr.id == k) {
                    return Err(format!("No criterion {k} to link to a part."));
                }
                out.push(k.to_string());
            }
            Some(out)
        }
    };
    let owner = role == Role::Owner;
    match id.and_then(|id| c.parts.iter().position(|p| p.id == id)) {
        Some(i) => {
            if remove {
                if owner {
                    return Err(format!("You can't remove part {}; only the lead or the person can.", c.parts[i].id));
                }
                c.parts.remove(i);
                return Ok(());
            }
            let cur = &mut c.parts[i];
            if let Some(t) = text.filter(|t| *t != cur.text) {
                if owner {
                    return Err(format!("You can't reword part {}; only the lead or the person can.", cur.id));
                }
                cur.text = t;
            }
            if let Some(links) = links {
                if owner {
                    if let Some(gone) = cur.criteria.iter().find(|k| !links.contains(k)) {
                        return Err(format!("You can't unlink criterion {gone} from part {}; only the lead or the person can.", cur.id));
                    }
                }
                cur.criteria = links;
            }
        }
        None => {
            if owner {
                return Err("Only the lead or the person can add a part of the request; you can link criteria to an existing part.".into());
            }
            if let Some(id) = id.filter(|_| text.is_none()) {
                return Err(format!("No part {id}; give text to add a new one."));
            }
            let Some(text) = text else { return Err("A new part needs text: the person's own words for it.".into()) };
            let used: Vec<&str> = c.parts.iter().map(|p| p.id.as_str()).collect();
            let id = next_id('p', &used);
            c.parts.push(Part { id, text, criteria: links.unwrap_or_default() });
        }
    }
    Ok(())
}

fn issue(c: &mut Contract, it: &Value, role: Role) -> Result<(), String> {
    let id = it["id"].as_str().map(str::trim).filter(|s| !s.is_empty());
    let text = it["text"].as_str().map(clip).filter(|s| !s.is_empty());
    let kind = it["kind"].as_str().map(str::trim).unwrap_or("open_issue");
    if !ISSUE_KINDS.contains(&kind) {
        return Err(format!("kind must be open_issue or assumption (got {kind})"));
    }
    let remove = it["remove"].as_bool() == Some(true);
    match id.and_then(|id| c.open_issues.iter().position(|x| x.id == id)) {
        Some(i) => {
            if role == Role::Owner && c.open_issues[i].by != "owner" {
                return Err(format!("You can't change open issue {} (the {} wrote it).", c.open_issues[i].id, c.open_issues[i].by));
            }
            if remove {
                c.open_issues.remove(i);
            } else {
                let cur = &mut c.open_issues[i];
                if let Some(t) = text {
                    cur.text = t;
                }
                cur.kind = kind.into();
            }
        }
        None => {
            let Some(text) = text else { return Err("A new open issue needs text.".into()) };
            let used: Vec<&str> = c.open_issues.iter().map(|x| x.id.as_str()).collect();
            c.open_issues.push(Issue { id: next_id('o', &used), kind: kind.into(), text, by: role.as_str().into() });
        }
    }
    Ok(())
}

/// Whether `c` is complete (contract §1.3) and, if not, why, in plain words. Evidence counts
/// only when it is still attached to the task.
pub fn completeness(c: &Contract, evidence_on_task: &[String]) -> (bool, Vec<String>) {
    let mut missing = Vec::new();
    if c.criteria.is_empty() {
        missing.push("no criteria".to_string());
    }
    for k in &c.criteria {
        match k.status.as_str() {
            "not_checked" => missing.push(format!("{} not checked", k.id)),
            "met" if !has_evidence(k, evidence_on_task) => missing.push(format!("{} met without evidence (a claim)", k.id)),
            _ => {}
        }
    }
    (missing.is_empty(), missing)
}

fn has_evidence(k: &Criterion, evidence_on_task: &[String]) -> bool {
    k.evidence.iter().any(|e| evidence_on_task.contains(e))
}

/// Whether `k` is a claim: `met` with no evidence on the task.
pub fn is_claim(k: &Criterion, evidence_on_task: &[String]) -> bool {
    k.status == "met" && !has_evidence(k, evidence_on_task)
}

/// The contract as the API shows it: each criterion with `claim`, plus `complete` and `missing`.
pub fn view(c: &Contract, evidence_on_task: &[String], updated_by: &str, updated_at: chrono::DateTime<chrono::Utc>) -> Value {
    let (complete, missing) = completeness(c, evidence_on_task);
    let criteria: Vec<Value> = c
        .criteria
        .iter()
        .map(|k| json!({"id": k.id, "text": k.text, "status": k.status, "evidence": k.evidence, "by": k.by, "round_added": k.round_added, "claim": is_claim(k, evidence_on_task)}))
        .collect();
    json!({
        "must_not_change": c.must_not_change,
        "criteria": criteria,
        "open_issues": c.open_issues,
        "risk_flags": c.risk_flags,
        "parts": c.parts,
        "complete": complete,
        "missing": missing,
        "updated_by": updated_by,
        "updated_at": updated_at,
    })
}

/// Why a reviewer may not pass a task with this contract: a criterion not met, or met without
/// evidence (contract §1.4). `None` when it may.
pub fn pass_blocker(c: &Contract, evidence_on_task: &[String]) -> Option<String> {
    if let Some(k) = c.criteria.iter().find(|k| k.status == "not_met") {
        return Some(format!("Criterion {} is not met: \"{}\". Send it back (fail) with what is missing.", k.id, k.text));
    }
    c.criteria
        .iter()
        .find(|k| is_claim(k, evidence_on_task))
        .map(|k| format!("Criterion {} has no evidence: it is a claim. Send it back (fail) with what is missing.", k.id))
}

/// The criteria for the lead's review prompt, one line each; a met without evidence reads
/// `CLAIM, no evidence`.
pub fn lead_lines(c: &Contract, evidence_on_task: &[String]) -> String {
    let mut out = Vec::new();
    for k in &c.criteria {
        let status = if is_claim(k, evidence_on_task) {
            "CLAIM, no evidence".to_string()
        } else if k.status == "met" {
            format!("met, evidence {}", k.evidence.join(", "))
        } else {
            k.status.replace('_', " ")
        };
        out.push(format!("- {} {}: {status}", k.id, k.text));
    }
    if !c.must_not_change.is_empty() {
        out.push(format!("Must not change: {}", c.must_not_change.join("; ")));
    }
    for i in &c.open_issues {
        out.push(format!("Open {} {}: {}", i.kind.replace('_', " "), i.id, i.text));
    }
    out.join("\n")
}

/// Whether a must_not_change entry names a path (contract §3.2, scope): no spaces, and a `/` or `.`.
pub fn path_like(entry: &str) -> bool {
    let e = entry.trim();
    !e.is_empty() && !e.contains(char::is_whitespace) && (e.contains('/') || e.contains('.'))
}

/// Each (entry, changed path) where a path-like must_not_change entry matches a changed path:
/// the same path, or the path under an entry ending in `/`.
pub fn scope_violations(must_not_change: &[String], changed: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for m in must_not_change.iter().filter(|m| path_like(m)) {
        let m = m.trim().trim_start_matches("./");
        for p in changed {
            let p2 = p.trim_start_matches("./");
            if p2 == m || (m.ends_with('/') && p2.starts_with(m)) {
                out.push((m.to_string(), p.clone()));
            }
        }
    }
    out
}

/// `[{id, status, claim}]` for every criterion, as a verdict stores it (contract §9.4).
pub fn criteria_status(c: &Contract, evidence_on_task: &[String]) -> Value {
    Value::Array(c.criteria.iter().map(|k| json!({"id": k.id, "status": k.status, "claim": is_claim(k, evidence_on_task)})).collect())
}

/// `[{entry, status}]` for each must_not_change entry (contract §9.10): a path entry is `met` or
/// `not_met` against the run's changed paths when there are some to compare (`changed` is
/// `Some`); everything else is `not_checked`, since only the person can judge behaviour.
pub fn scope_status(must_not_change: &[String], changed: Option<&[String]>) -> Value {
    Value::Array(
        must_not_change
            .iter()
            .map(|m| {
                let status = match changed {
                    Some(ch) if path_like(m) => {
                        if scope_violations(std::slice::from_ref(m), ch).is_empty() {
                            "met"
                        } else {
                            "not_met"
                        }
                    }
                    _ => "not_checked",
                };
                json!({"entry": m, "status": status})
            })
            .collect(),
    )
}

// Loading and saving

/// The task's stored contract, if any.
pub async fn load(state: &AppState, task_id: &str) -> Option<(Contract, String, chrono::DateTime<chrono::Utc>)> {
    let row = state.db.get_contract(task_id).await.ok().flatten()?;
    let c = serde_json::from_str::<Contract>(&row.body).ok()?;
    Some((c, row.updated_by, row.updated_at))
}

pub async fn evidence_ids(state: &AppState, task_id: &str) -> Vec<String> {
    state.db.list_evidence(task_id).await.unwrap_or_default().into_iter().map(|e| e.id).collect()
}

/// The `contract` key of the task's JSON, or null.
pub async fn json_of(state: &AppState, task_id: &str) -> Value {
    match load(state, task_id).await {
        Some((c, by, at)) => view(&c, &evidence_ids(state, task_id).await, &by, at),
        None => Value::Null,
    }
}

/// The bot that leads `task`: the lead of its hand-off, else of its mission.
pub async fn lead_of(state: &AppState, task: &Task) -> Option<String> {
    if task.source_kind == "delegation" {
        if let Some(id) = task.source_id.as_deref() {
            return state.db.get_delegation(id).await.ok().flatten().map(|d| d.lead_kwant_id);
        }
    }
    let mid = task.mission_id.as_deref()?;
    state.db.get_mission(mid).await.ok().flatten().map(|m| m.lead_kwant_id)
}

/// Writes `body` to `task`'s contract as `role` (logged as `by`). A task with no stored contract
/// starts from its acceptance lines.
pub async fn write(state: &AppState, task: &Task, body: &Value, role: Role, by: &str) -> Result<Value, String> {
    let ev = evidence_ids(state, &task.id).await;
    let current = match load(state, &task.id).await {
        Some((c, _, _)) => c,
        None => Contract { criteria: from_acceptance(&task.acceptance), ..Default::default() },
    };
    let round = state.db.verdicts_of(&task.id).await.map(|v| v.len() as i64).unwrap_or(0) + 1;
    let (next, fields) = apply_in_round(current, body, role, &ev, round)?;
    let event = json!({"by": role.as_str(), "fields": fields, "criteria": next.criteria.len(), "must_not_change": next.must_not_change.len()});
    let body = serde_json::to_value(&next).map_err(|e| e.to_string())?;
    let row = state.db.put_contract(&task.id, &body, by, &event).await.map_err(|e| e.to_string())?;
    super::emit_task_id(state, &task.id).await;
    Ok(view(&next, &ev, &row.updated_by, row.updated_at))
}

/// The person's send-back: its reason becomes a criterion (V1, "the note becomes a criterion").
pub async fn add_send_back(state: &AppState, task: &Task, reason: &str) {
    let body = json!({"criteria": [{"text": reason, "status": "not_checked"}]});
    if let Err(e) = write(state, task, &body, Role::Person, "person").await {
        tracing::info!(task_id = %task.id, error = %e, "the send-back note didn't become a criterion");
    }
}

/// What `parts` asks for, in `task_contract` and `mission_plan`.
pub const PARTS_HELP: &str = "Cut the person's request into parts that together cover every word of it, in their words; link each part to the criteria that prove it.";

/// One `parts` entry, for `task_contract` and `mission_plan`.
pub fn part_schema() -> Value {
    json!({"type": "object", "properties": {
        "id": {"type": "string", "description": "An existing part's id (p1, p2…); leave out to add one."},
        "text": {"type": "string", "description": "The person's own words for this part, e.g. \"send it to Ana\"."},
        "criteria": {"type": "array", "items": {"type": "string"}, "description": "Ids of the criteria that prove this part (c1, c2…)."},
        "remove": {"type": "boolean"}
    }})
}

pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Write a task's contract: what must not change, its criteria (each met / not_met / not_checked, with evidence ids attached to the task), open issues or assumptions, and risk flags. The task's owner can add items and mark criteria with evidence, but can't remove or reword what the lead wrote; the lead of the hand-off or mission can set anything. A criterion marked met without evidence counts as a claim, not as met. Parts: cut the person's request into parts that together cover every word of it, in their words; link each part to the criteria that prove it. Only the lead or the person writes parts; the owner can link more criteria to one. A part with no criterion linked shows the person it has no check.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "must_not_change": {"type": "array", "items": {"type": "string"}, "description": "The whole list: paths (src/db/x.rs, docs/) or behaviour that must stay as it is."},
                "criteria": {"type": "array", "items": {"type": "object", "properties": {
                    "id": {"type": "string", "description": "An existing criterion's id (c1, c2…); leave out to add one."},
                    "text": {"type": "string"},
                    "status": {"type": "string", "enum": STATUSES},
                    "evidence": {"type": "array", "items": {"type": "string"}, "description": "Evidence ids attached to this task."},
                    "remove": {"type": "boolean"}
                }}},
                "open_issues": {"type": "array", "items": {"type": "object", "properties": {
                    "id": {"type": "string"}, "kind": {"type": "string", "enum": ISSUE_KINDS}, "text": {"type": "string"}, "remove": {"type": "boolean"}
                }}},
                "risk_flags": {"type": "array", "items": {"type": "string"}, "description": "The whole list: short tags like migration, auth, payments."},
                "parts": {"type": "array", "maxItems": MAX_PARTS, "description": PARTS_HELP, "items": part_schema()}
            },
            "required": ["task_id"]
        }
    })
}

/// `task_contract` for `kwant_id` on `task` (already checked to be in its space).
pub async fn call(state: &AppState, kwant_id: &str, task: &Task, args: &Value) -> (String, bool) {
    let role = if lead_of(state, task).await.as_deref() == Some(kwant_id) {
        Role::Lead
    } else if task.owner_kwant_id.as_deref() == Some(kwant_id) {
        Role::Owner
    } else {
        return ("Only the task's owner or the bot that leads it can write its contract.".into(), true);
    };
    if matches!(task.state.as_str(), "verified" | "cancelled") {
        return (format!("The task is {}; its contract is closed.", task.state), true);
    }
    match write(state, task, args, role, kwant_id).await {
        Ok(v) => (serde_json::to_string_pretty(&json!({"contract": v})).unwrap_or_default(), false),
        Err(e) => (e, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev() -> Vec<String> {
        vec!["e1".into(), "e2".into()]
    }

    fn lead_contract() -> Contract {
        let body = json!({"must_not_change": ["src/db/missions.rs"], "risk_flags": ["Migration"], "criteria": [{"text": "inbox lists review tasks"}, {"text": "phone layout"}]});
        apply(Contract::default(), &body, Role::Lead, &ev()).unwrap().0
    }

    #[test]
    fn acceptance_lines_become_criteria() {
        let c = from_acceptance("- one\n\n* two\n  three  \n");
        assert_eq!(c.iter().map(|k| (k.id.as_str(), k.text.as_str())).collect::<Vec<_>>(), [("c1", "one"), ("c2", "two"), ("c3", "three")]);
        assert!(c.iter().all(|k| k.status == "not_checked" && k.by == "acceptance"));
        assert_eq!(from_acceptance(&"x\n".repeat(30)).len(), MAX_FROM_ACCEPTANCE);
    }

    #[test]
    fn the_lead_sets_anything_and_flags_are_lowercase() {
        let c = lead_contract();
        assert_eq!(c.risk_flags, ["migration"]);
        assert_eq!(c.criteria[1].id, "c2");
        assert_eq!(c.criteria[0].by, "lead");
        let (c, _) = apply(c, &json!({"must_not_change": [], "criteria": [{"id": "c2", "remove": true}]}), Role::Lead, &ev()).unwrap();
        assert!(c.must_not_change.is_empty() && c.criteria.len() == 1);
    }

    #[test]
    fn the_owner_can_only_add_and_mark() {
        let c = lead_contract();
        let e = apply(c.clone(), &json!({"must_not_change": []}), Role::Owner, &ev()).unwrap_err();
        assert!(e.contains("src/db/missions.rs"), "{e}");
        assert!(apply(c.clone(), &json!({"risk_flags": []}), Role::Owner, &ev()).unwrap_err().contains("migration"));
        assert!(apply(c.clone(), &json!({"criteria": [{"id": "c1", "text": "something easier"}]}), Role::Owner, &ev()).unwrap_err().contains("reword"));
        assert!(apply(c.clone(), &json!({"criteria": [{"id": "c1", "remove": true}]}), Role::Owner, &ev()).unwrap_err().contains("remove"));
        let (c, fields) = apply(
            c,
            &json!({"must_not_change": ["src/db/missions.rs", "docs/"], "criteria": [{"id": "c1", "status": "met", "evidence": ["e1"]}, {"text": "my own"}], "open_issues": [{"text": "assumes one inbox", "kind": "assumption"}]}),
            Role::Owner,
            &ev(),
        )
        .unwrap();
        assert_eq!(fields, ["must_not_change", "criteria", "open_issues"]);
        assert_eq!((c.criteria[0].status.as_str(), c.criteria[2].by.as_str(), c.criteria[2].id.as_str()), ("met", "owner", "c3"));
        assert_eq!(c.open_issues[0].id, "o1");
        // Its own criterion it may reword
        let (c, _) = apply(c, &json!({"criteria": [{"id": "c3", "text": "my own, clearer"}]}), Role::Owner, &ev()).unwrap();
        assert_eq!(c.criteria[2].text, "my own, clearer");
    }

    #[test]
    fn evidence_must_be_on_the_task_and_values_are_checked() {
        let c = lead_contract();
        assert!(apply(c.clone(), &json!({"criteria": [{"id": "c1", "status": "met", "evidence": ["nope"]}]}), Role::Owner, &ev()).unwrap_err().contains("nope"));
        assert!(apply(c.clone(), &json!({"criteria": [{"id": "c1", "status": "done"}]}), Role::Owner, &ev()).is_err());
        assert!(apply(c.clone(), &json!({"criteria": [{"id": "c9", "status": "met"}]}), Role::Owner, &ev()).unwrap_err().contains("c9"));
        assert!(apply(c.clone(), &json!({}), Role::Lead, &ev()).is_err(), "nothing to change");
        let many: Vec<String> = (0..9).map(|i| format!("f{i}")).collect();
        assert!(apply(c, &json!({"risk_flags": many}), Role::Lead, &ev()).is_err());
    }

    #[test]
    fn a_met_without_evidence_is_a_claim() {
        let (c, _) = apply(lead_contract(), &json!({"criteria": [{"id": "c1", "status": "met"}, {"id": "c2", "status": "met", "evidence": ["e2"]}]}), Role::Owner, &ev()).unwrap();
        let (complete, missing) = completeness(&c, &ev());
        assert!(!complete);
        assert_eq!(missing, ["c1 met without evidence (a claim)"]);
        assert!(is_claim(&c.criteria[0], &ev()) && !is_claim(&c.criteria[1], &ev()));
        // Evidence that was removed from the task no longer counts
        assert!(is_claim(&c.criteria[1], &["e1".to_string()]));
        let v = view(&c, &ev(), "k", chrono::Utc::now());
        assert_eq!((v["criteria"][0]["claim"].as_bool(), v["complete"].as_bool()), (Some(true), Some(false)));
        assert!(pass_blocker(&c, &ev()).unwrap().contains("c1 has no evidence"));
        let lines = lead_lines(&c, &ev());
        assert!(lines.contains("c1 inbox lists review tasks: CLAIM, no evidence"), "{lines}");
        assert!(lines.contains("c2 phone layout: met, evidence e2"));
        assert!(lines.contains("Must not change: src/db/missions.rs"));
    }

    #[test]
    fn complete_needs_every_criterion_checked() {
        assert_eq!(completeness(&Contract::default(), &ev()).1, ["no criteria"]);
        let c = lead_contract();
        assert_eq!(completeness(&c, &ev()).1, ["c1 not checked", "c2 not checked"]);
        let (c, _) = apply(c, &json!({"criteria": [{"id": "c1", "status": "met", "evidence": ["e1"]}, {"id": "c2", "status": "not_met"}]}), Role::Owner, &ev()).unwrap();
        assert!(completeness(&c, &ev()).0);
        assert!(pass_blocker(&c, &ev()).unwrap().contains("c2 is not met"));
    }

    #[test]
    fn scope_matches_paths_and_folders_only() {
        assert!(path_like("src/db/missions.rs") && path_like("docs/") && path_like("Cargo.toml"));
        assert!(!path_like("the public API") && !path_like("auth") && !path_like(""));
        let mnc = vec!["src/db/missions.rs".to_string(), "docs/".into(), "the public API".into()];
        let changed = vec!["src/db/missions.rs".to_string(), "docs/a.md".into(), "src/db/missions.rs.bak".into(), "src/x.rs".into()];
        let v = scope_violations(&mnc, &changed);
        assert_eq!(v, [("src/db/missions.rs".to_string(), "src/db/missions.rs".to_string()), ("docs/".into(), "docs/a.md".into())]);
    }

    #[test]
    fn round_added_is_set_once_by_kwant() {
        let c = lead_contract();
        assert!(c.criteria.iter().all(|k| k.round_added == 1));
        let (c, _) = apply_in_round(c, &json!({"criteria": [{"text": "the send-back note"}, {"id": "c1", "status": "met", "evidence": ["e1"], "round_added": 7}]}), Role::Person, &ev(), 2).unwrap();
        assert_eq!(c.criteria.iter().map(|k| k.round_added).collect::<Vec<_>>(), [1, 1, 2], "a write can't change it; new ones get the round");
        let old: Criterion = serde_json::from_value(json!({"id": "c1", "text": "t", "status": "met", "by": "lead"})).unwrap();
        assert_eq!(old.round_added, 1, "criteria stored before the field read as round 1");
        assert_eq!(view(&c, &ev(), "lead", chrono::Utc::now())["criteria"][2]["round_added"], 2);
    }

    #[test]
    fn verdicts_record_criteria_and_scope_status() {
        let (c, _) = apply(lead_contract(), &json!({"criteria": [{"id": "c1", "status": "met"}, {"id": "c2", "status": "met", "evidence": ["e2"]}]}), Role::Owner, &ev()).unwrap();
        assert_eq!(criteria_status(&c, &ev()), json!([{"id": "c1", "status": "met", "claim": true}, {"id": "c2", "status": "met", "claim": false}]));
        let mnc = vec!["src/db/missions.rs".to_string(), "docs/".to_string(), "the login flow".to_string()];
        let changed = vec!["docs/a.md".to_string(), "src/x.rs".to_string()];
        assert_eq!(
            scope_status(&mnc, Some(&changed)),
            json!([{"entry": "src/db/missions.rs", "status": "met"}, {"entry": "docs/", "status": "not_met"}, {"entry": "the login flow", "status": "not_checked"}])
        );
        assert!(scope_status(&mnc, None).as_array().unwrap().iter().all(|e| e["status"] == "not_checked"));
    }

    // ---- request-parts ----

    fn with_parts() -> Contract {
        let body = json!({"criteria": [{"text": "the PDF is in the folder"}, {"text": "Ana has the email"}], "parts": [{"text": "make the PDF", "criteria": ["c1"]}, {"text": "send it to Ana"}]});
        apply(Contract::default(), &body, Role::Lead, &ev()).unwrap().0
    }

    #[test]
    fn parts_round_trip_and_an_old_body_has_none() {
        let c = with_parts();
        assert_eq!(c.parts, [Part { id: "p1".into(), text: "make the PDF".into(), criteria: vec!["c1".into()] }, Part { id: "p2".into(), text: "send it to Ana".into(), criteria: vec![] }]);
        let back: Contract = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        let old: Contract = serde_json::from_str(r#"{"must_not_change": [], "criteria": [{"id": "c1", "text": "t", "status": "met", "by": "lead"}], "open_issues": [], "risk_flags": []}"#).unwrap();
        assert!(old.parts.is_empty());
        assert_eq!(view(&c, &ev(), "lead", chrono::Utc::now())["parts"][1]["text"], "send it to Ana");
    }

    #[test]
    fn the_owner_can_link_criteria_to_a_part_but_not_change_parts() {
        let c = with_parts();
        let refuse = |body: Value, want: &str| {
            let e = apply(c.clone(), &body, Role::Owner, &ev()).unwrap_err();
            assert!(e.contains(want), "{e}");
        };
        refuse(json!({"parts": [{"id": "p2", "remove": true}]}), "can't remove part p2");
        refuse(json!({"parts": [{"id": "p2", "text": "send it"}]}), "can't reword part p2");
        refuse(json!({"parts": [{"text": "and tell Bob"}]}), "Only the lead or the person");
        refuse(json!({"parts": [{"id": "p1", "criteria": []}]}), "can't unlink criterion c1");
        // It may add its own criterion and link it
        let (c2, fields) = apply(c.clone(), &json!({"criteria": [{"text": "sent, message id in evidence"}], "parts": [{"id": "p2", "criteria": ["c3"]}]}), Role::Owner, &ev()).unwrap();
        assert_eq!(fields, ["criteria", "parts"]);
        assert_eq!(c2.parts[1].criteria, ["c3"]);
        // The person and the lead may do anything
        let (c3, _) = apply(c, &json!({"parts": [{"id": "p1", "text": "make the PDF, A4", "criteria": []}, {"id": "p2", "remove": true}]}), Role::Person, &ev()).unwrap();
        assert_eq!(c3.parts, [Part { id: "p1".into(), text: "make the PDF, A4".into(), criteria: vec![] }]);
    }

    #[test]
    fn parts_link_only_existing_criteria_and_lose_removed_ones() {
        let c = with_parts();
        assert!(apply(c.clone(), &json!({"parts": [{"id": "p2", "criteria": ["c9"]}]}), Role::Lead, &ev()).unwrap_err().contains("c9"));
        assert!(apply(c.clone(), &json!({"parts": [{"criteria": ["c1"]}]}), Role::Lead, &ev()).unwrap_err().contains("needs text"));
        assert!(apply(c.clone(), &json!({"parts": "send it"}), Role::Lead, &ev()).is_err());
        let many: Vec<Value> = (0..MAX_PARTS).map(|i| json!({"text": format!("part {i}")})).collect();
        assert!(apply(c.clone(), &json!({"parts": many}), Role::Lead, &ev()).unwrap_err().contains("at most"));
        let (c, _) = apply(c, &json!({"criteria": [{"id": "c1", "remove": true}]}), Role::Lead, &ev()).unwrap();
        assert!(c.parts[0].criteria.is_empty(), "a removed criterion is unlinked");
        // Parts in a plan: c1 is the first acceptance line
        let start = Contract { criteria: from_acceptance("PDF in the folder\nAna has it"), ..Default::default() };
        let (c, _) = apply(start, &json!({"parts": [{"text": "make the PDF", "criteria": ["c1"]}, {"text": "send it to Ana", "criteria": ["c2"]}]}), Role::Lead, &[]).unwrap();
        assert_eq!(c.parts[1].criteria, ["c2"]);
    }
}
