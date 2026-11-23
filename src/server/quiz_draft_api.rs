//! pa4c-server: the person's own edits to a quiz draft's questions, on the main port. Every path
//! is person-only in `auth::guard` (a bot's turn token gets 403), and a pocket's guest or host key
//! means nothing here. While the `pocket_apps` switch is off every route is 404 `pocket_apps_off`.
//! Docs: autonomy/pocket-apps/pa4c-server/impl.md, "Draft edit API".
//!
//! - `GET    /api/quiz-drafts/:id?space=`                → `{draft, card}`
//! - `POST   /api/quiz-drafts/:id/questions`             `{round, question, answer, accepted?, options?, kind?, base?}` → `{draft, card}`
//! - `PATCH  /api/quiz-drafts/:id/questions/:n`          `{question?, answer?, accepted?, options?, base?}` → `{draft, card}`
//! - `DELETE /api/quiz-drafts/:id/questions/:n?base=`    → `{draft, card}`
//!
//! `n` counts every question from 1 across rounds. An edit goes the way a bot's `pocket_update`
//! does (validator, pa3's rules, the 256 KB cap, every problem numbered at once, then the source
//! check and the 30-player dry run), with three differences: a question the person added or
//! changed becomes theirs (`source: {"own": true}`, kept in `quiz_drafts.own_keys`) and isn't
//! source-checked; every earlier source result is reused, so nothing is fetched again; and an edit
//! with problems is refused (422) with nothing saved. `base` is the draft's `updated_at` the
//! person saw: when a bot updated it since, the edit is refused (409 `changed`).

use super::quizmaster_tools::{card_of, loose_questions, own_keys, recheck};
use super::spaces::Scope;
use super::AppState;
use crate::db::quiz_drafts::QuizDraftRow;
use crate::kits::quiz::validate;
use crate::quizmaster::card::{self, DryRunLine, State};
use crate::quizmaster::source_check::{self, SourceResult};
use axum::body::Bytes;
use axum::extract::{Path, Query, State as AxState};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/quiz-drafts/:id", get(read))
        .route("/api/quiz-drafts/:id/questions", post(add))
        .route("/api/quiz-drafts/:id/questions/:n", patch(edit).delete(remove))
}

/// A refusal: `{error, message, ...extra}`.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    extra: Value,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), extra: json!({}) }
    }
    fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_input", message)
    }
}

impl From<crate::db::DbError> for ApiError {
    fn from(e: crate::db::DbError) -> Self {
        tracing::warn!("quiz draft edit: database error: {e}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Something went wrong saving the quiz. Try again.")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.code, "message": self.message });
        if let (Some(b), Some(extra)) = (body.as_object_mut(), self.extra.as_object()) {
            b.extend(extra.clone());
        }
        (self.status, Json(body)).into_response()
    }
}

type R<T> = Result<T, ApiError>;

/// The draft in the person's space, while pocket apps are on.
async fn load(state: &AppState, space: &str, id: &str) -> R<QuizDraftRow> {
    if !crate::pocket::enabled(&state.db).await {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "pocket_apps_off", "Pocket apps are off. Turn them on in Settings first."));
    }
    state.db.quiz_draft_get(space, id).await?.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "There's no quiz draft here."))
}

/// Only a draft that isn't open, closed or still being written takes the person's edits.
fn editable(row: &QuizDraftRow) -> R<()> {
    match State::parse(&row.state) {
        Some(State::Open) => Err(ApiError::new(StatusCode::CONFLICT, "quiz_open", "An open quiz's questions can't change.")),
        Some(State::Closed) => Err(ApiError::new(StatusCode::CONFLICT, "quiz_closed", "A closed quiz's questions can't change.")),
        Some(State::Writing) | None => Err(ApiError::new(
            StatusCode::CONFLICT,
            "still_writing",
            "The bot is still writing this quiz. Its questions can be changed once it shows the card.",
        )),
        _ => Ok(()),
    }
}

fn changed() -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "changed", "The quiz changed since you opened it. Open it again to see the new version.")
}

/// `base` as sent (the draft's `updated_at` the person saw); a different one is refused.
fn check_base(row: &QuizDraftRow, base: Option<&Value>) -> R<()> {
    match base {
        None | Some(Value::Null) => Ok(()),
        Some(b) => match b.as_i64().or_else(|| b.as_str().and_then(|s| s.parse().ok())) {
            Some(b) if b == row.updated_at => Ok(()),
            Some(_) => Err(changed()),
            None => Err(ApiError::bad("`base` must be the draft's `updated_at`.")),
        },
    }
}

fn parse<T: serde::de::DeserializeOwned + Default>(s: &str) -> T {
    serde_json::from_str(s).unwrap_or_default()
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

/// The kind a question is, as written or as its fields say.
fn kind_of(q: &Value) -> &'static str {
    match q["kind"].as_str() {
        Some("choice") => "choice",
        Some("number") => "number",
        Some("text") => "text",
        _ if q.get("options").is_some_and(Value::is_array) => "choice",
        _ if q["answer"].is_number() => "number",
        _ => "text",
    }
}

/// The draft as the edit screen shows it.
pub fn view(row: &QuizDraftRow) -> Value {
    let own = own_keys(row);
    let sources: Vec<SourceResult> = parse(&row.sources);
    let spec: Value = serde_json::from_str(&row.spec).unwrap_or(Value::Null);
    let st = State::parse(&row.state).unwrap_or(State::Writing);
    let rounds: Vec<Value> = spec["rounds"].as_array().into_iter().flatten().enumerate().map(|(i, r)| json!({ "n": i + 1, "title": r["title"] })).collect();
    let round_of: Vec<usize> = spec["rounds"].as_array().into_iter().flatten().enumerate().flat_map(|(i, r)| std::iter::repeat_n(i + 1, r["questions"].as_array().map_or(0, Vec::len))).collect();
    let (mut checked, mut yours, mut not_checked) = (0, 0, 0);
    let mut questions = Vec::new();
    for (n, (at, v, q)) in loose_questions(&row.spec).into_iter().enumerate() {
        let mut item = json!({
            "n": n + 1,
            "round": round_of.get(n).copied().unwrap_or(1),
            "at": at,
            "kind": kind_of(&v),
            "question": v["text"],
            "answer": v["answer"],
            "accepted": v.get("accept").filter(|a| a.is_array()).cloned().unwrap_or_else(|| json!([])),
        });
        if let Some(o) = v.get("options").filter(|o| o.is_array()) {
            item["options"] = o.clone();
        }
        if let Some(p) = v.get("picture").filter(|p| p.is_object()) {
            item["picture"] = p.clone();
        }
        let result = sources.iter().find(|r| r.q == n);
        if q.as_ref().is_some_and(|q| source_check::is_own(q, &own)) {
            yours += 1;
            item["status"] = json!("yours");
        } else if let Some(r) = result.filter(|r| r.ok) {
            checked += 1;
            item["status"] = json!("checked");
            item["source_url"] = json!(r.url);
        } else {
            not_checked += 1;
            item["status"] = json!("not_checked");
            if let Some(r) = result {
                let why = r.fail.map(|f| f.says().to_string()).unwrap_or_default();
                item["why"] = json!(match &r.detail {
                    Some(d) => format!("{why} ({d})"),
                    None => why,
                });
            }
        }
        questions.push(item);
    }
    let total = questions.len();
    let mut header = format!("{} · {checked} checked · {yours} yours", plural(total, "question"));
    if not_checked > 0 {
        header.push_str(&format!(" · {not_checked} not checked"));
    }
    let dry: Option<DryRunLine> = row.dry_run.as_deref().and_then(|d| serde_json::from_str(d).ok());
    json!({
        "id": row.id,
        "title": row.title,
        "state": st.as_str(),
        "chip": st.chip(),
        "kwant_id": row.kwant_id,
        "pocket_code": row.pocket_code,
        "updated_at": row.updated_at,
        "counts": { "questions": total, "checked": checked, "yours": yours, "not_checked": not_checked },
        "header": header,
        "dry_run": dry,
        "problems": parse::<Vec<String>>(&row.problems),
        "rounds": rounds,
        "questions": questions,
    })
}

fn reply(row: &QuizDraftRow) -> Json<Value> {
    Json(json!({ "draft": view(row), "card": card_of(row) }))
}

async fn read(AxState(state): AxState<AppState>, Scope(space): Scope, Path(id): Path<String>) -> R<Json<Value>> {
    let row = load(&state, space.as_str(), &id).await?;
    Ok(reply(&row))
}

/// The JSON object a request sent.
fn body_of(bytes: &Bytes) -> R<Map<String, Value>> {
    if bytes.is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_slice(bytes) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(ApiError::bad("The body must be a JSON object.")),
    }
}

fn text_field(b: &Map<String, Value>, key: &str) -> R<Option<String>> {
    match b.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(ApiError::bad(format!("`{key}` must be text."))),
    }
}

fn list_field(b: &Map<String, Value>, key: &str) -> R<Option<Vec<String>>> {
    match b.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().map(|s| s.trim().to_string()).ok_or_else(|| ApiError::bad(format!("`{key}` must be a list of text."))))
            .collect::<R<Vec<_>>>()
            .map(|v| Some(v.into_iter().filter(|s| !s.is_empty()).collect())),
        Some(_) => Err(ApiError::bad(format!("`{key}` must be a list of text."))),
    }
}

/// The answer as the question's kind stores it: a number for a number question.
fn answer_field(b: &Map<String, Value>, kind: &str) -> R<Option<Value>> {
    match b.get("answer") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if kind == "number" => Ok(Some(s.trim().parse::<f64>().ok().and_then(|f| serde_json::Number::from_f64(f).map(Value::Number)).unwrap_or_else(|| json!(s.trim())))),
        Some(Value::String(s)) => Ok(Some(json!(s.trim()))),
        Some(n @ Value::Number(_)) => Ok(Some(n.clone())),
        Some(_) => Err(ApiError::bad("`answer` must be text or a number.")),
    }
}

/// Sets the fields the person sent on a question.
fn apply(q: &mut Value, b: &Map<String, Value>) -> R<()> {
    let kind = kind_of(q);
    if let Some(t) = text_field(b, "question")? {
        q["text"] = json!(t);
    }
    if let Some(a) = answer_field(b, kind)? {
        q["answer"] = a;
    }
    if let Some(o) = list_field(b, "options")? {
        q["options"] = json!(o);
    }
    if let Some(acc) = list_field(b, "accepted")? {
        let obj = q.as_object_mut().ok_or_else(|| ApiError::bad("That question can't be read."))?;
        if acc.is_empty() {
            obj.remove("accept");
        } else {
            obj.insert("accept".into(), json!(acc));
        }
    }
    Ok(())
}

/// Every problem a spec has, numbered at once (the validator's, else pa3's rules).
fn problems_of(spec_text: &str) -> Vec<String> {
    match validate::parse(spec_text) {
        Err(p) => p.iter().map(ToString::to_string).collect(),
        Ok(spec) => card::rule_problems(&spec).into_iter().enumerate().map(|(i, p)| format!("{}. {p}", i + 1)).collect(),
    }
}

/// The question at `n` (1-based, across rounds): `(round index, index in round)`.
fn locate(spec: &Value, n: usize) -> Option<(usize, usize)> {
    let mut left = n.checked_sub(1)?;
    for (r, round) in spec["rounds"].as_array()?.iter().enumerate() {
        let len = round["questions"].as_array().map_or(0, Vec::len);
        if left < len {
            return Some((r, left));
        }
        left -= len;
    }
    None
}

fn no_question(n: usize) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "no_question", format!("This quiz has no question {n}."))
}

/// Checks and saves the person's new spec. `mine`: the question they added or changed, now theirs.
async fn save(state: &AppState, mut row: QuizDraftRow, spec: Value, mine: Option<Value>) -> R<Json<Value>> {
    let spec_text = spec.to_string();
    let problems = problems_of(&spec_text);
    if !problems.is_empty() {
        let mut e = ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid", "Nothing was saved: fix these first.");
        e.extra = json!({ "problems": problems });
        return Err(e);
    }
    let mut own = own_keys(&row);
    if let Some(q) = mine.and_then(|q| serde_json::from_value(q).ok()) {
        own.insert(source_check::own_key(&q));
    }
    let mut keys: Vec<&String> = own.iter().collect();
    keys.sort();
    row.own_keys = serde_json::to_string(&keys).unwrap_or_else(|_| "[]".into());
    // Every earlier result stands for a question that didn't change: nothing is fetched again
    let cache: HashMap<String, SourceResult> = parse::<Vec<SourceResult>>(&row.sources).into_iter().map(|r| (r.key.clone(), r)).collect();
    let base = row.updated_at;
    recheck(&mut row, spec_text, &cache, false).await;
    if row.updated_at <= base {
        row.updated_at = base + 1;
    }
    if !state.db.quiz_draft_save_if(&row, Some(base)).await? {
        return Err(changed());
    }
    Ok(reply(&row))
}

fn spec_of(row: &QuizDraftRow) -> R<Value> {
    serde_json::from_str::<Value>(&row.spec).ok().filter(|v| v["rounds"].is_array()).ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "still_writing", "This quiz can't be read yet."))
}

async fn edit(AxState(state): AxState<AppState>, Scope(space): Scope, Path((id, n)): Path<(String, usize)>, body: Bytes) -> R<Json<Value>> {
    let row = load(&state, space.as_str(), &id).await?;
    let b = body_of(&body)?;
    editable(&row)?;
    check_base(&row, b.get("base"))?;
    let mut spec = spec_of(&row)?;
    let (r, i) = locate(&spec, n).ok_or_else(|| no_question(n))?;
    let q = &mut spec["rounds"][r]["questions"][i];
    let before = q.clone();
    apply(q, &b)?;
    if *q == before {
        return Ok(reply(&row));
    }
    q["source"] = json!({ "own": true });
    let mine = q.clone();
    save(&state, row, spec, Some(mine)).await
}

async fn add(AxState(state): AxState<AppState>, Scope(space): Scope, Path(id): Path<String>, body: Bytes) -> R<Json<Value>> {
    let row = load(&state, space.as_str(), &id).await?;
    let b = body_of(&body)?;
    editable(&row)?;
    check_base(&row, b.get("base"))?;
    let mut spec = spec_of(&row)?;
    let rounds = spec["rounds"].as_array().map_or(0, Vec::len);
    let round = match b.get("round") {
        None | Some(Value::Null) => rounds,
        Some(v) => v.as_u64().map(|r| r as usize).ok_or_else(|| ApiError::bad("`round` must be a round number, from 1."))?,
    };
    if round == 0 || round > rounds {
        return Err(ApiError::bad(format!("This quiz has no round {round}.")));
    }
    let kind = match text_field(&b, "kind")?.as_deref() {
        Some(k @ ("choice" | "text" | "number")) => k.to_string(),
        Some(_) => return Err(ApiError::bad("`kind` must be choice, text or number.")),
        None if b.get("options").is_some_and(Value::is_array) => "choice".into(),
        None if b.get("answer").is_some_and(Value::is_number) => "number".into(),
        None => "text".into(),
    };
    let mut q = json!({ "kind": kind, "text": "" });
    apply(&mut q, &b)?;
    q["source"] = json!({ "own": true });
    let list = spec["rounds"][round - 1]["questions"].as_array_mut().ok_or_else(|| ApiError::bad(format!("Round {round} can't be read.")))?;
    list.push(q.clone());
    save(&state, row, spec, Some(q)).await
}

async fn remove(AxState(state): AxState<AppState>, Scope(space): Scope, Path((id, n)): Path<(String, usize)>, Query(q): Query<HashMap<String, String>>) -> R<Json<Value>> {
    let row = load(&state, space.as_str(), &id).await?;
    editable(&row)?;
    check_base(&row, q.get("base").map(|b| json!(b)).as_ref())?;
    let mut spec = spec_of(&row)?;
    let (r, i) = locate(&spec, n).ok_or_else(|| no_question(n))?;
    if let Some(list) = spec["rounds"][r]["questions"].as_array_mut() {
        list.remove(i);
    }
    save(&state, row, spec, None).await
}

/// The keys a question set holds (used by tests in this module).
#[cfg(test)]
fn keys_of(spec: &Value) -> std::collections::HashSet<String> {
    loose_questions(&spec.to_string()).into_iter().filter_map(|(_, _, q)| q).map(|q| source_check::own_key(&q)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_are_counted_across_rounds() {
        let spec = json!({ "rounds": [{ "questions": [{}, {}] }, { "questions": [{}] }] });
        assert_eq!(locate(&spec, 1), Some((0, 0)));
        assert_eq!(locate(&spec, 2), Some((0, 1)));
        assert_eq!(locate(&spec, 3), Some((1, 0)));
        assert_eq!(locate(&spec, 4), None);
        assert_eq!(locate(&spec, 0), None);
    }

    #[test]
    fn edits_set_only_what_was_sent() {
        let mut q = json!({ "kind": "number", "text": "When?", "answer": 1989, "source": { "url": "https://x", "quote": "1989" } });
        let b: Map<String, Value> = serde_json::from_value(json!({ "answer": "1990" })).unwrap();
        apply(&mut q, &b).unwrap();
        assert_eq!(q["answer"], json!(1990.0), "a number question keeps a number");
        assert_eq!(q["text"], json!("When?"));
        let b: Map<String, Value> = serde_json::from_value(json!({ "answer": { "x": 1 } })).unwrap();
        assert!(apply(&mut q, &b).is_err());
        let mut t = json!({ "kind": "text", "text": "Who?", "answer": "Zidane", "accept": ["Zizou"] });
        let b: Map<String, Value> = serde_json::from_value(json!({ "accepted": [" ", ""] })).unwrap();
        apply(&mut t, &b).unwrap();
        assert!(t.get("accept").is_none(), "an empty list removes the spellings");
    }

    #[test]
    fn own_keys_follow_what_the_person_wrote() {
        let a = json!({ "rounds": [{ "questions": [{ "kind": "text", "text": "Who?", "answer": "Zidane", "source": { "own": true } }] }] });
        let mut b = a.clone();
        b["rounds"][0]["questions"][0]["points"] = json!(2);
        assert_eq!(keys_of(&a), keys_of(&b), "points aren't what the person wrote");
        b["rounds"][0]["questions"][0]["answer"] = json!("Zizou");
        assert_ne!(keys_of(&a), keys_of(&b));
    }
}
