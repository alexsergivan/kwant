//! Skills over HTTP and at turn time. Agents propose skills with `skill_propose` (stored
//! `pending`), the user reviews the diff and approves, edits, disables or rejects them, and a
//! turn gets only the enabled skills that fit its prompt, never the whole library. A skill is
//! text: nothing in it changes which tools a turn has or what needs approval.

// Handler helpers return the ready-made error `Response`
#![allow(clippy::result_large_err)]

use super::continuity::{notify, Notification, SelfScope};
use super::spaces::Scope;
use super::AppState;
use crate::db::skills::{clean_skill_text, Skill, SkillEdit, SkillError, SKILL_STATUSES};
use crate::drivers::events::RuntimeEvent;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Most skill bodies put into one prompt.
pub const MAX_SKILLS_IN_PROMPT: usize = 2;
/// Most bytes the injected skill bodies may take together.
pub const SKILL_PROMPT_BYTES: usize = 6 * 1024;
/// Most other skills named in the prompt's index (so the agent can load one by name).
pub const SKILL_INDEX_ENTRIES: usize = 15;
/// A skill matched only by shared words needs at least this score to be injected.
const MIN_RELEVANCE: u32 = 3;
/// Most lines a diff compares on each side (bodies are capped well below this).
const MAX_DIFF_LINES: usize = 2000;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn skill_error(e: SkillError) -> Response {
    match e {
        SkillError::NotFound => StatusCode::NOT_FOUND.into_response(),
        SkillError::Conflict(why) => error(StatusCode::CONFLICT, why),
        SkillError::Db(detail) => {
            tracing::warn!(error = %detail, "skill update failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        other => error(StatusCode::BAD_REQUEST, other.to_string()),
    }
}

fn changed(state: &AppState, skill: &Skill) {
    let _ = state.event_sender.send(RuntimeEvent::SkillUpdated {
        skill: serde_json::to_value(skill).unwrap_or(Value::Null),
    });
}

// Selection

const STOPWORDS: [&str; 40] = [
    "the", "and", "for", "with", "that", "this", "you", "your", "are", "was", "can", "how", "what", "when", "who",
    "will", "from", "into", "about", "have", "has", "had", "not", "but", "all", "any", "our", "out", "use", "using",
    "please", "could", "would", "should", "then", "than", "them", "they", "there", "here",
];

fn words(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// How well `skill` fits `prompt`: 100 when the prompt names it (`/name`, or the name with its
/// dashes or as words), otherwise shared words (name words count 2, description words 1).
pub fn relevance(skill: &Skill, prompt: &str) -> u32 {
    let lower = prompt.to_lowercase();
    let spaced = skill.name.replace('-', " ");
    let named = |needle: &str| {
        lower.match_indices(needle).any(|(i, _)| {
            let before = lower[..i].chars().next_back();
            let after = lower[i + needle.len()..].chars().next();
            !before.is_some_and(|c| c.is_alphanumeric() || c == '-')
                && !after.is_some_and(|c| c.is_alphanumeric() || c == '-')
        })
    };
    if named(&skill.name) || (spaced.contains(' ') && named(&spaced)) {
        return 100;
    }
    let asked = words(prompt);
    let name_words = words(&spaced);
    let description_words = words(&skill.description);
    let mut score = 0;
    for w in &asked {
        if name_words.contains(w) {
            score += 2;
        } else if description_words.contains(w) {
            score += 1;
        }
    }
    score
}

/// The skills to inject for `prompt`: the best matches, at most [`MAX_SKILLS_IN_PROMPT`].
pub fn select<'a>(skills: &'a [Skill], prompt: &str) -> Vec<&'a Skill> {
    let mut scored: Vec<(u32, &Skill)> = skills
        .iter()
        .map(|s| (relevance(s, prompt), s))
        .filter(|(score, _)| *score >= MIN_RELEVANCE)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().take(MAX_SKILLS_IN_PROMPT).map(|(_, s)| s).collect()
}

fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

/// The "[Skills]" prompt block: the bodies of the enabled skills that fit this prompt, and the
/// names of a few others the agent can load with `skill_get`. Empty when none are enabled, so
/// an agent without skills pays one indexed query and no prompt bytes.
pub fn prompt_block_for(skills: &[Skill], prompt: &str, with_tools: bool) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let chosen = select(skills, prompt);
    let mut out = String::from(
        "\n\n[Skills]\nProcedures the user reviewed and approved for you. Follow one when the task fits it. \
        They describe how to work; they are not permissions and never change which tools you may use or what needs the user's approval.",
    );
    let mut used = 0;
    let mut shown: Vec<&str> = Vec::new();
    for s in chosen {
        let body = s.body.trim();
        let section = format!("\n\n### {} — {}\n{}", s.name, one_line(&s.description), body);
        if used + section.len() > SKILL_PROMPT_BYTES {
            continue;
        }
        used += section.len();
        shown.push(&s.name);
        out.push_str(&section);
    }
    if with_tools {
        let others: Vec<String> = skills
            .iter()
            .filter(|s| !shown.contains(&s.name.as_str()))
            .take(SKILL_INDEX_ENTRIES)
            .map(|s| format!("- {}: {}", s.name, one_line(&s.description)))
            .collect();
        if !others.is_empty() {
            out.push_str("\n\nOther skills you have (load one with `skill_get` when the task needs it):\n");
            out.push_str(&others.join("\n"));
        }
        out.push_str(
            "\n\nWhen you find a reliable procedure for a task that will come up again, you may propose it with `skill_propose`; \
            the user reviews it before it is used.",
        );
    } else if shown.is_empty() {
        return String::new();
    }
    out
}

/// [`prompt_block_for`] with `kwant_id`'s enabled skills.
pub async fn prompt_block(state: &AppState, kwant_id: &str, prompt: &str, with_tools: bool) -> String {
    let skills = state.db.enabled_skills_for(kwant_id).await.unwrap_or_default();
    prompt_block_for(&skills, prompt, with_tools)
}

// Diff

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DiffLine {
    /// "same", "add" or "del".
    pub op: &'static str,
    pub text: String,
}

/// A line diff of `old` → `new` (longest common subsequence).
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().take(MAX_DIFF_LINES).collect();
    let b: Vec<&str> = new.lines().take(MAX_DIFF_LINES).collect();
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(n.max(m));
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(DiffLine { op: "same", text: a[i].into() });
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(DiffLine { op: "del", text: a[i].into() });
            i += 1;
        } else {
            out.push(DiffLine { op: "add", text: b[j].into() });
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|l| DiffLine { op: "del", text: (*l).into() }));
    out.extend(b[j..].iter().map(|l| DiffLine { op: "add", text: (*l).into() }));
    out
}

fn unified(lines: &[DiffLine]) -> String {
    lines
        .iter()
        .map(|l| {
            let mark = match l.op {
                "add" => '+',
                "del" => '-',
                _ => ' ',
            };
            format!("{mark}{}\n", l.text)
        })
        .collect()
}

// Routes

/// The space a skill request is about: the kwant's, or the request's (`?space=`) when it names
/// none.
async fn skill_space(state: &AppState, kwant_id: Option<&str>, scope: Scope) -> crate::db::spaces::SpaceId {
    match kwant_id {
        Some(id) => match state.db.space_of_kwant(id).await {
            Ok(Some(space)) => space,
            _ => scope.0,
        },
        None => scope.0,
    }
}

#[derive(Deserialize)]
pub struct SkillsQuery {
    pub kwant_id: Option<String>,
    pub status: Option<String>,
}

/// `GET /api/skills?kwant_id=&status=`: newest first; `kwant_id` includes the shared skills.
pub async fn list(State(state): State<AppState>, scope: Scope, Query(q): Query<SkillsQuery>) -> Response {
    let status = q.status.filter(|s| !s.is_empty());
    if let Some(s) = &status {
        if !SKILL_STATUSES.contains(&s.as_str()) {
            return error(StatusCode::BAD_REQUEST, format!("unknown status '{s}'"));
        }
    }
    let kwant_id = q.kwant_id.as_deref().filter(|s| !s.is_empty());
    let space = skill_space(&state, kwant_id, scope).await;
    match state.db.list_skills(&space, kwant_id, status.as_deref()).await {
        Ok(list) => Json(list).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.get_skill(&id).await {
        Ok(Some(s)) => Json(s).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// `GET /api/skills/:id/diff`: what approving would change. The base is the skill a revision
/// replaces, or nothing for a new skill.
pub async fn diff(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let skill = match state.db.get_skill(&id).await {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let base = match &skill.replaces_id {
        Some(old) => state.db.get_skill(old).await.ok().flatten(),
        None => None,
    };
    let lines = diff_lines(base.as_ref().map_or("", |b| b.body.as_str()), &skill.body);
    Json(json!({
        "skill": skill,
        "base": base,
        "description_changed": base.as_ref().is_some_and(|b| b.description != skill.description),
        "lines": lines,
        "unified": unified(&lines),
    }))
    .into_response()
}

/// `kwant_id` in a payload: absent leaves the owner, `null` shares the skill with every agent.
fn deserialize_some<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

#[derive(Deserialize, Default)]
pub struct SkillPayload {
    pub name: Option<String>,
    pub description: Option<String>,
    pub body: Option<String>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub kwant_id: Option<Option<String>>,
    /// PATCH only: true enables, false disables a reviewed skill.
    pub enabled: Option<bool>,
}

async fn check_owner(state: &AppState, kwant_id: &Option<Option<String>>) -> Result<(), Response> {
    if let Some(Some(id)) = kwant_id {
        match state.db.get_kwant(id).await {
            Ok(Some(_)) => {}
            Ok(None) => return Err(error(StatusCode::BAD_REQUEST, "unknown kwant_id")),
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        }
    }
    Ok(())
}

impl SkillPayload {
    fn edit(&self) -> SkillEdit {
        SkillEdit {
            name: self.name.clone(),
            description: self.description.clone(),
            body: self.body.clone(),
            kwant_id: self.kwant_id.clone(),
        }
    }
}

/// `POST /api/skills {name, description, body, kwant_id?}`: a skill the user wrote, enabled at once.
pub async fn create(
    State(state): State<AppState>,
    scope: Scope,
    uri: axum::http::Uri,
    Json(p): Json<SkillPayload>,
) -> Response {
    if let Err(r) = check_owner(&state, &p.kwant_id).await {
        return r;
    }
    // With an explicit ?space=, the owner must be in it (as for routines); without, the skill
    // follows its owner
    if let (true, Some(Some(owner))) = (super::spaces::space_param(uri.query()).is_some(), &p.kwant_id) {
        match state.db.space_of_kwant(owner).await {
            Ok(Some(of)) if of == scope.0 => {}
            Ok(_) => return error(StatusCode::BAD_REQUEST, "that bot is in another space"),
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
    let text = match clean_skill_text(
        p.name.as_deref().unwrap_or(""),
        p.description.as_deref().unwrap_or(""),
        p.body.as_deref().unwrap_or(""),
    ) {
        Ok(t) => t,
        Err(e) => return skill_error(e),
    };
    let kwant_id = p.kwant_id.flatten();
    let space = skill_space(&state, kwant_id.as_deref(), scope).await;
    if let Err(r) = super::spaces::ensure_active(&state, &space).await {
        return r;
    }
    match state.db.create_user_skill(&space, kwant_id.as_deref(), text).await {
        Ok(s) => {
            changed(&state, &s);
            (StatusCode::CREATED, Json(s)).into_response()
        }
        Err(e) => skill_error(e),
    }
}

/// `PATCH /api/skills/:id {name?, description?, body?, kwant_id?, enabled?}`.
pub async fn update(State(state): State<AppState>, Path(id): Path<String>, Json(p): Json<SkillPayload>) -> Response {
    if let Err(r) = check_owner(&state, &p.kwant_id).await {
        return r;
    }
    let edit = p.edit();
    let mut skill = None;
    if edit.name.is_some() || edit.description.is_some() || edit.body.is_some() || edit.kwant_id.is_some() {
        match state.db.edit_skill(&id, &edit).await {
            Ok(s) => skill = Some(s),
            Err(e) => return skill_error(e),
        }
    }
    if let Some(on) = p.enabled {
        match state.db.set_skill_status(&id, if on { "enabled" } else { "disabled" }).await {
            Ok(s) => skill = Some(s),
            Err(e) => return skill_error(e),
        }
    }
    let skill = match skill {
        Some(s) => s,
        None => match state.db.get_skill(&id).await {
            Ok(Some(s)) => s,
            Ok(None) => return StatusCode::NOT_FOUND.into_response(),
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
    };
    changed(&state, &skill);
    Json(skill).into_response()
}

/// `POST /api/skills/:id/approve {name?, description?, body?, kwant_id?}`: enables a pending
/// skill, with the user's edits if given.
pub async fn approve(State(state): State<AppState>, Path(id): Path<String>, body: Option<Json<SkillPayload>>) -> Response {
    let p = body.map(|Json(p)| p).unwrap_or_default();
    if let Err(r) = check_owner(&state, &p.kwant_id).await {
        return r;
    }
    match state.db.approve_skill(&id, &p.edit()).await {
        Ok(s) => {
            if let Some(old) = &s.replaces_id {
                if let Ok(Some(o)) = state.db.get_skill(old).await {
                    changed(&state, &o);
                }
            }
            changed(&state, &s);
            Json(s).into_response()
        }
        Err(e) => skill_error(e),
    }
}

/// `POST /api/skills/:id/reject`: a pending skill is turned down (kept for the record).
pub async fn reject(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.set_skill_status(&id, "rejected").await {
        Ok(s) => {
            changed(&state, &s);
            Json(s).into_response()
        }
        Err(e) => skill_error(e),
    }
}

pub async fn remove(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let skill = match state.db.get_skill(&id).await {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    match state.db.delete_skill(&id).await {
        Ok(true) => {
            let _ = state.event_sender.send(RuntimeEvent::SkillUpdated {
                skill: json!({"id": skill.id, "kwant_id": skill.kwant_id, "space_id": skill.space_id, "deleted": true}),
            });
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// Agent tools

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "skill_propose",
            "description": "Propose a skill: a short written procedure for a task that will come up again, taken from something \
                that worked. The user reviews it before it is used; until then nothing changes. A skill is instructions only; \
                it can't give you tools or permissions. Proposing a name you already have proposes a new version of it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Short lowercase name with dashes, like release-checklist"},
                    "description": {"type": "string", "description": "One line: when to use it (max 200 characters)"},
                    "body": {"type": "string", "description": "The steps, in Markdown (max 8000 characters). No secrets."},
                },
                "required": ["name", "description", "body"],
            },
        }),
        json!({
            "name": "skill_get",
            "description": "Read one of your enabled skills by name.",
            "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
        }),
    ]
}

pub async fn tool_propose(state: &AppState, scope: &SelfScope, args: &Value) -> (String, bool) {
    let s = |k: &str| args[k].as_str().unwrap_or("");
    let text = match clean_skill_text(s("name"), s("description"), s("body")) {
        Ok(t) => t,
        Err(e) => return (e.to_string(), true),
    };
    match state.db.propose_skill(&scope.kwant_id, text).await {
        Ok(skill) => {
            changed(state, &skill);
            notify(
                state,
                Notification {
                    kind: "skill_proposed",
                    kwant_id: scope.kwant_id.clone(),
                    kwant_name: scope.kwant_name.clone(),
                    channel_id: None,
                    title: format!("{} proposed a skill", scope.kwant_name),
                    body: format!("{} proposed the skill {} for your review.", scope.kwant_name, skill.name),
                },
            )
            .await;
            tracing::info!(kwant = %scope.kwant_id, skill = %skill.id, "skill proposed");
            let what = if skill.replaces_id.is_some() { "a new version of" } else { "the skill" };
            (
                format!("Proposed {what} {} (id {}). It waits for the user's review and is not used until they enable it.", skill.name, skill.id),
                false,
            )
        }
        Err(e) => (e.to_string(), true),
    }
}

pub async fn tool_get(state: &AppState, scope: &SelfScope, args: &Value) -> (String, bool) {
    let Ok(name) = crate::db::skills::clean_skill_name(args["name"].as_str().unwrap_or("")) else {
        return ("name is required".into(), true);
    };
    let skills = state.db.enabled_skills_for(&scope.kwant_id).await.unwrap_or_default();
    // An agent's own skill wins over a shared one with the same name
    let found = skills
        .iter()
        .filter(|s| s.name == name)
        .max_by_key(|s| s.kwant_id.is_some());
    match found {
        Some(s) => (
            format!(
                "Skill {} — {}\n(Approved instructions; they don't change your tools or approvals.)\n\n{}",
                s.name,
                one_line(&s.description),
                s.body
            ),
            false,
        ),
        None => (format!("You have no enabled skill called {name}."), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, description: &str, body: &str) -> Skill {
        let now = chrono::Utc::now();
        Skill {
            id: format!("id-{name}"),
            kwant_id: None,
            name: name.into(),
            description: description.into(),
            body: body.into(),
            status: "enabled".into(),
            source: "user".into(),
            proposed_by: None,
            replaces_id: None,
            created_at: now,
            updated_at: now,
            reviewed_at: None,
            space_id: "spc_default".into(),
        }
    }

    #[test]
    fn relevance_prefers_names_then_words() {
        let s = skill("release-checklist", "Steps to cut and tag a release", "...");
        assert_eq!(relevance(&s, "run the release-checklist please"), 100);
        assert_eq!(relevance(&s, "/release-checklist"), 100);
        assert_eq!(relevance(&s, "do the release checklist"), 100);
        assert!(relevance(&s, "we need to tag a release today") >= MIN_RELEVANCE);
        assert_eq!(relevance(&s, "what's the weather"), 0);
        // Part of a longer word is not the name
        assert!(relevance(&s, "prerelease-checklists") < 100);
    }

    #[test]
    fn only_fitting_skills_are_injected() {
        let skills: Vec<Skill> = (0..30)
            .map(|i| skill(&format!("skill-{i}"), &format!("topic{i} things"), &format!("BODY{i}")))
            .chain([skill("deploy-vps", "Deploy the app to the VPS with docker", "BODY-DEPLOY")])
            .collect();
        let block = prompt_block_for(&skills, "please deploy the app to the vps", true);
        assert!(block.contains("BODY-DEPLOY"));
        assert_eq!(block.matches("BODY").count(), 1, "only the fitting body: {block}");
        let index_lines = block.lines().filter(|l| l.starts_with("- skill-")).count();
        assert_eq!(index_lines, SKILL_INDEX_ENTRIES, "the index is capped");
        assert!(block.contains("not permissions"));
        // Without tools and without a match there's nothing to say
        assert_eq!(prompt_block_for(&skills, "hello there", false), "");
        assert_eq!(prompt_block_for(&[], "deploy", true), "");
    }

    #[test]
    fn injected_bodies_stay_within_budget() {
        let big = "x".repeat(5000);
        let skills = vec![skill("alpha-deploy", "deploy", &big), skill("beta-deploy", "deploy", &big)];
        let block = prompt_block_for(&skills, "alpha-deploy and beta-deploy", false);
        assert!(block.len() < SKILL_PROMPT_BYTES + 400);
        assert_eq!(block.matches("### ").count(), 1);
    }

    #[test]
    fn diff_marks_changes() {
        let d = diff_lines("a\nb\nc", "a\nc\nd");
        let ops: Vec<_> = d.iter().map(|l| (l.op, l.text.as_str())).collect();
        assert_eq!(ops, vec![("same", "a"), ("del", "b"), ("same", "c"), ("add", "d")]);
        assert_eq!(unified(&d), " a\n-b\n c\n+d\n");
        assert!(diff_lines("", "x").iter().all(|l| l.op == "add"));
    }
}
