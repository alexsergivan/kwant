//! pa3 Quizmaster: the bot's Pocket tools (create, update, dry run, open, close) and the judge
//! hook's model call. Offered only while the `pocket_apps` setting is `on` (off by default), and
//! refused when it is off.
//!
//! The tools work on a quiz draft (`quiz_drafts`) owned by the bot that wrote it. Each call that
//! hands in a spec validates it (every problem, numbered, at once), checks every source, runs the
//! dry run and answers with the Pocket card. `pocket_open` works only on a Ready card and goes
//! through [`PocketCore`], the smallest hook pa1 needs to register (see pa3/impl.md, "For Lin").

use super::continuity::SelfScope;
use super::{profile_turn, AppState};
use crate::db::models::Kwant;
use crate::db::quiz_drafts::QuizDraftRow;
use crate::drivers::engines::{choose_model, known_models, Tier};
use crate::drivers::TurnInput;
use crate::quizmaster::budget::{judge_allowance, Budget};
use crate::quizmaster::card::{self, DryRunLine, State};
use crate::quizmaster::judge::{Judge, JudgeBudget, JudgeModel};
use crate::quizmaster::source_check::{self, Policy, SourceResult};
use crate::quizmaster::{self as qm, preset};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

type ToolResult = (String, bool);

/// The switch (pa1's `pocket::SETTING`).
pub const SETTING: &str = "pocket_apps";
pub const TOOLS: [&str; 5] = ["pocket_create", "pocket_update", "pocket_dry_run", "pocket_open", "pocket_close"];
/// The largest spec a bot may hand in (the Quiz validator's limit).
const MAX_SPEC_BYTES: usize = crate::kits::quiz::validate::MAX_SPEC_BYTES;

pub fn handles(name: &str) -> bool {
    TOOLS.contains(&name)
}

/// Whether pocket apps are on: the Pocket core's own check (pa3 qa F6), so both read the switch
/// the same way.
pub async fn enabled(state: &AppState) -> bool {
    crate::pocket::enabled(&state.db).await
}

/// One lock per quiz draft (pa3 qa F2): every read → check → save of a draft runs under it, so
/// two calls on one quiz (two opens at once, an update racing an open, the live judge writing
/// its count) take turns. Locks nobody holds or waits for are dropped on the next call.
pub async fn draft_lock(id: &str) -> tokio::sync::OwnedMutexGuard<()> {
    static LOCKS: OnceLock<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let lock = {
        let mut map = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, l| Arc::strong_count(l) > 1);
        map.entry(id.to_string()).or_default().clone()
    };
    lock.lock_owned().await
}

/// The row as stored now (read again under [`draft_lock`]).
async fn fresh(state: &AppState, row: &QuizDraftRow) -> Result<QuizDraftRow, ToolResult> {
    match state.db.quiz_draft_get(&row.space_id, &row.id).await {
        Ok(Some(r)) => Ok(r),
        Ok(None) => Err(fail("That quiz is gone.")),
        Err(e) => {
            tracing::warn!(error = %e, "quiz draft read failed");
            Err(fail("Couldn't read the quiz."))
        }
    }
}

static FAIL_OPEN_SAVE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Tests: the next `pocket_open` of this draft fails to save it after the pocket opened.
#[cfg(feature = "test-seams")]
#[doc(hidden)]
pub fn fail_next_open_save_for_tests(draft_id: &str) {
    *FAIL_OPEN_SAVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(draft_id.to_string());
}

fn fail_this_open_save(draft_id: &str) -> bool {
    let mut f = FAIL_OPEN_SAVE.lock().unwrap_or_else(|e| e.into_inner());
    if f.as_deref() == Some(draft_id) {
        *f = None;
        return true;
    }
    false
}

/// The chat a pocket was asked for in (pa7 `CreateRequest.origin`): its reminder card and results
/// post go there. Built on the server from the turn's own context, never from tool arguments, so a
/// bot can't aim a pocket's posts at another chat.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PocketOrigin {
    pub kwant_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

impl PocketOrigin {
    pub fn of(scope: &SelfScope) -> Self {
        let o = scope.origin.as_ref();
        Self {
            kwant_id: scope.kwant_id.clone(),
            channel_id: o.and_then(|o| o.channel_id.clone()),
            thread_id: o.and_then(|o| o.thread_id.clone()),
        }
    }
}

/// What pa1 registers so a Ready quiz can open as a pocket, move and close. Errors are plain
/// sentences for the bot (pa7's `bad_origin`, `ended`, `bad_times` mapped to words).
#[async_trait]
pub trait PocketCore: Send + Sync {
    /// Creates the pocket (pa1 `pocket::create`, with pa7's `origin`) and returns its code.
    async fn open(&self, state: &AppState, kit: &str, spec: &Value, origin: &PocketOrigin) -> Result<String, String>;
    /// Ends the pocket.
    async fn close(&self, state: &AppState, code: &str) -> Result<(), String>;
    /// Moves the event (pa7 `pocket::schedule::set_when`): an RFC 3339 time, or `None` for no time.
    async fn set_when(&self, _state: &AppState, _code: &str, _when: Option<&str>) -> Result<(), String> {
        Err("this kwant build can't move an open pocket yet".into())
    }
}

static CORE: OnceLock<Arc<dyn PocketCore>> = OnceLock::new();

/// pa1 calls this once at start-up. False when one is already registered.
pub fn register_pocket_core(core: Arc<dyn PocketCore>) -> bool {
    CORE.set(core).is_ok()
}

fn pocket_core() -> Option<&'static Arc<dyn PocketCore>> {
    CORE.get()
}

pub fn tools() -> Vec<Value> {
    let id = json!({"type": "string", "description": "The quiz's id from its card (qd_…)"});
    let spec = json!({"type": "object", "description": "The whole pocket.json (kit \"quiz\", version 1), written from the template pocket_create gives you"});
    vec![
        json!({
            "name": "pocket_create",
            "description": "Make a quiz pocket app (a pub quiz, class quiz or party quiz guests play on their phones). Call it first with no spec to get \
                the template and the writing rules. Then call it with the whole spec: it validates it, checks every answer against its source page, \
                runs a dry run with 30 pretend players and returns the Pocket card. Show the person the card only when it says Ready.",
            "inputSchema": {"type": "object", "properties": {"spec": spec}},
        }),
        json!({
            "name": "pocket_update",
            "description": "Hand in a fixed or changed quiz: the whole spec again. It is checked like pocket_create (unchanged sources aren't fetched again) \
                and returns the card. To move the event only (also once it's open), send `when` instead of `spec`.",
            "inputSchema": {"type": "object", "properties": {"id": id, "spec": spec,
                "when": {"type": ["string", "null"], "description": "The new start time, RFC 3339 with its offset (2026-11-27T21:00:00+00:00), or null for no time"}},
                "required": ["id"]},
        }),
        json!({
            "name": "pocket_dry_run",
            "description": "Run the quiz again with pretend players (default 30, at most 60) and return the card.",
            "inputSchema": {"type": "object", "properties": {"id": id, "players": {"type": "integer", "minimum": 2, "maximum": 60}}, "required": ["id"]},
        }),
        json!({
            "name": "pocket_open",
            "description": "Open a Ready quiz so guests can join (only when the person wants it shared).",
            "inputSchema": {"type": "object", "properties": {"id": id}, "required": ["id"]},
        }),
        json!({
            "name": "pocket_close",
            "description": "Close an open quiz.",
            "inputSchema": {"type": "object", "properties": {"id": id}, "required": ["id"]},
        }),
    ]
}

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The spec argument as JSON text: an object, or a string holding one.
fn spec_arg(args: &Value) -> Result<Option<String>, String> {
    let text = match args.get("spec") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => {
            let v: Value = serde_json::from_str(s).map_err(|e| format!("`spec` isn't valid JSON: {e}"))?;
            if !v.is_object() {
                return Err("`spec` must be a JSON object".into());
            }
            v.to_string()
        }
        Some(v @ Value::Object(_)) => v.to_string(),
        Some(_) => return Err("`spec` must be a JSON object".into()),
    };
    if text.len() > MAX_SPEC_BYTES {
        return Err(format!("the spec is {} KB; the most is {} KB", text.len() / 1024, MAX_SPEC_BYTES / 1024));
    }
    Ok(Some(text))
}

fn id_arg(args: &Value) -> Result<&str, String> {
    args.get("id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty() && s.len() <= 64).ok_or_else(|| "`id` is required".to_string())
}

pub async fn call(state: &AppState, scope: &SelfScope, name: &str, args: &Value) -> ToolResult {
    if !enabled(state).await {
        return fail("Pocket apps are off on this kwant.");
    }
    let space = match state.db.space_of_kwant(&scope.kwant_id).await {
        Ok(Some(s)) => s.as_str().to_string(),
        _ => return fail("Could not find your space."),
    };
    match name {
        "pocket_create" => create(state, scope, &space, args).await,
        "pocket_update" | "pocket_dry_run" | "pocket_open" | "pocket_close" => {
            let id = match id_arg(args) {
                Ok(id) => id,
                Err(e) => return fail(e),
            };
            let row = match state.db.quiz_draft_get(&space, id).await {
                Ok(Some(r)) if r.kwant_id == scope.kwant_id => r,
                Ok(Some(_)) => return fail("That quiz belongs to another bot."),
                Ok(None) => return fail(format!("There's no quiz {id:?} in your space.")),
                Err(e) => {
                    tracing::warn!(error = %e, "quiz draft read failed");
                    return fail("Couldn't read the quiz.");
                }
            };
            match name {
                "pocket_update" => update(state, row, args).await,
                "pocket_dry_run" => dry_run(state, row, args).await,
                "pocket_open" => open(state, row, &PocketOrigin::of(scope)).await,
                _ => close(state, row).await,
            }
        }
        _ => fail(format!("Unknown tool {name}.")),
    }
}

fn template() -> String {
    format!(
        "{}\n\nTemplate (a complete, valid quiz; its quotes are samples, so write your own from pages you name):\n{}\n\nCall pocket_create again with `spec` set to your whole quiz.",
        preset::SKILL.trim(),
        crate::kits::quiz::EXAMPLE.trim()
    )
}

fn parse<T: serde::de::DeserializeOwned + Default>(s: &str) -> T {
    serde_json::from_str(s).unwrap_or_default()
}

/// Builds the card for a row.
pub(crate) fn card_of(row: &QuizDraftRow) -> card::Card {
    let spec = crate::kits::quiz::validate::parse(&row.spec).ok();
    let problems: Vec<String> = parse(&row.problems);
    let sources: Vec<SourceResult> = parse(&row.sources);
    let dry: Option<DryRunLine> = row.dry_run.as_deref().and_then(|d| serde_json::from_str(d).ok());
    let budget: Budget = parse(&row.budget);
    card::build(card::CardInput {
        id: &row.id,
        state: State::parse(&row.state).unwrap_or(State::Writing),
        spec: spec.as_ref(),
        title: &row.title,
        problems: &problems,
        sources: &sources,
        dry_run: dry.as_ref(),
        budget: &budget,
        pocket: row.pocket_code.as_deref(),
    })
}

async fn save_and_show(state: &AppState, row: &QuizDraftRow) -> ToolResult {
    save_if_and_show(state, row, None).await
}

/// [`save_and_show`], saving only while the stored draft's `updated_at` is still `base` (pa4c).
async fn save_if_and_show(state: &AppState, row: &QuizDraftRow, base: Option<i64>) -> ToolResult {
    match state.db.quiz_draft_save_if(row, base).await {
        Ok(true) => {}
        Ok(false) => return fail("The person changed this quiz while you were working, so nothing was saved. Call pocket_update again: it tells you which of their questions to keep."),
        Err(e) => {
            tracing::warn!(error = %e, "quiz draft save failed");
            return fail("Couldn't save the quiz.");
        }
    }
    let c = card_of(row);
    let is_error = matches!(c.state, State::Writing);
    (c.text(), is_error)
}

/// pa4c: the person's own question keys stored on a draft.
pub(crate) fn own_keys(row: &QuizDraftRow) -> HashSet<String> {
    parse(&row.own_keys)
}

/// pa4c: each question of a spec that may not validate, read one by one, with where it sits
/// ("round 1, question 2") and its JSON. A question that doesn't read is left out.
pub(crate) fn loose_questions(spec_text: &str) -> Vec<(String, Value, Option<crate::kits::quiz::spec::Question>)> {
    let v: Value = serde_json::from_str(spec_text).unwrap_or(Value::Null);
    let mut out = Vec::new();
    for (r, round) in v["rounds"].as_array().into_iter().flatten().enumerate() {
        for (i, q) in round["questions"].as_array().into_iter().flatten().enumerate() {
            out.push((format!("round {}, question {}", r + 1, i + 1), q.clone(), serde_json::from_value(q.clone()).ok()));
        }
    }
    out
}

/// pa4c: the person's own keys still present in a spec.
fn own_keys_in(spec_text: &str, own: &HashSet<String>) -> HashSet<String> {
    loose_questions(spec_text).into_iter().filter_map(|(_, _, q)| q).filter(|q| source_check::is_own(q, own)).map(|q| source_check::own_key(&q)).collect()
}

/// Checks a spec and writes the result onto `row`, counting one writing call. The person's own
/// questions (pa4c) skip the source check; keys no longer in the spec are dropped.
pub(crate) async fn check_into(row: &mut QuizDraftRow, spec_text: String, cache: &HashMap<String, SourceResult>) {
    recheck(row, spec_text, cache, true).await
}

/// [`check_into`]; `writing_call`: whether it counts as one of the bot's writing calls (the
/// person's own edits, pa4c, don't).
pub(crate) async fn recheck(row: &mut QuizDraftRow, spec_text: String, cache: &HashMap<String, SourceResult>, writing_call: bool) {
    let own = own_keys_in(&spec_text, &own_keys(row));
    let e = qm::evaluate_own(&spec_text, cache, &Policy::current(), &own).await;
    let mut own: Vec<String> = own.into_iter().collect();
    own.sort();
    row.own_keys = serde_json::to_string(&own).unwrap_or_else(|_| "[]".into());
    let mut budget: Budget = parse(&row.budget);
    if writing_call {
        budget.writing_calls += 1;
    }
    budget.source_fetches += e.fetched;
    if let Some(s) = &e.spec {
        budget.judge_allowance = judge_allowance(qm::text_questions(s));
        row.title = s.title.trim().chars().take(80).collect();
    }
    row.spec = spec_text;
    row.state = e.state.as_str().into();
    row.problems = serde_json::to_string(&e.problems).unwrap_or_else(|_| "[]".into());
    row.sources = serde_json::to_string(&e.sources).unwrap_or_else(|_| "[]".into());
    row.dry_run = e.dry_run.as_ref().and_then(|d| serde_json::to_string(d).ok());
    row.budget = serde_json::to_string(&budget).unwrap_or_else(|_| "{}".into());
    row.updated_at = now_ms();
}

async fn create(state: &AppState, scope: &SelfScope, space: &str, args: &Value) -> ToolResult {
    let spec_text = match spec_arg(args) {
        Ok(Some(s)) => s,
        Ok(None) => return (template(), false),
        Err(e) => return fail(e),
    };
    let now = now_ms();
    let mut row = QuizDraftRow {
        id: format!("qd_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
        space_id: space.to_string(),
        kwant_id: scope.kwant_id.clone(),
        title: String::new(),
        spec: String::new(),
        state: State::Writing.as_str().into(),
        problems: "[]".into(),
        sources: "[]".into(),
        dry_run: None,
        budget: "{}".into(),
        pocket_code: None,
        created_at: now,
        updated_at: now,
        own_keys: "[]".into(),
    };
    check_into(&mut row, spec_text, &HashMap::new()).await;
    save_and_show(state, &row).await
}

/// The `when` argument: `Some(None)` clears the time, `Some(Some(t))` sets it (RFC 3339, kept as
/// written), `None` when it's absent.
fn when_arg(args: &Value) -> Result<Option<Option<String>>, String> {
    match args.get("when") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.len() > 64 || chrono::DateTime::parse_from_rfc3339(t).is_err() {
                return Err("`when` must be an RFC 3339 time with its offset, like 2026-11-27T21:00:00+00:00, or null".into());
            }
            Ok(Some(Some(t.to_string())))
        }
        Some(_) => Err("`when` must be a string or null".into()),
    }
}

/// The stored spec with `when` set or removed.
fn spec_with_when(spec: &str, when: Option<&str>) -> Option<String> {
    let mut v: Value = serde_json::from_str(spec).ok()?;
    let o = v.as_object_mut()?;
    match when {
        Some(t) => o.insert("when".into(), json!(t)),
        None => o.remove("when"),
    };
    Some(v.to_string())
}

async fn update(state: &AppState, row: QuizDraftRow, args: &Value) -> ToolResult {
    let _g = draft_lock(&row.id).await;
    let mut row = match fresh(state, &row).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let st = State::parse(&row.state);
    let when = match when_arg(args) {
        Ok(w) => w,
        Err(e) => return fail(e),
    };
    let spec_text = match spec_arg(args) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    if spec_text.is_some() && when.is_some() {
        return fail("Send either the whole `spec` (with its `when` inside) or `when` alone, not both.");
    }
    if let (None, Some(when)) = (&spec_text, &when) {
        return move_when(state, row, when.as_deref()).await;
    }
    if matches!(st, Some(State::Open | State::Closed)) {
        return fail("This quiz is open or closed, so its questions can't change: guests may be playing it. Make a new one with pocket_create.");
    }
    let Some(spec_text) = spec_text else {
        return fail("`spec` is required: the whole quiz, not only the changes.");
    };
    // pa4c: the questions the person wrote stay as they wrote them
    if let Some(msg) = drops_persons_questions(&row, &spec_text) {
        return fail(msg);
    }
    // Passed checks are reused while the question's source and answer stay the same
    let cache: HashMap<String, SourceResult> = parse::<Vec<SourceResult>>(&row.sources).into_iter().filter(|r| r.ok).map(|r| (r.key.clone(), r)).collect();
    let base = row.updated_at;
    check_into(&mut row, spec_text, &cache).await;
    // pa4c: a person's edit that landed while this update ran wins; the bot calls again
    save_if_and_show(state, &row, Some(base)).await
}

/// pa4c: a bot's whole-spec update that drops or changes a question the person wrote, said with
/// those questions' JSON so the bot can keep them as they are. `None`: every one is still there.
fn drops_persons_questions(row: &QuizDraftRow, spec_text: &str) -> Option<String> {
    let own = own_keys(row);
    if own.is_empty() {
        return None;
    }
    let kept = own_keys_in(spec_text, &own);
    let lost: Vec<(String, Value)> = loose_questions(&row.spec)
        .into_iter()
        .filter_map(|(at, v, q)| q.filter(|q| source_check::is_own(q, &own) && !kept.contains(&source_check::own_key(q))).map(|_| (at, v)))
        .collect();
    if lost.is_empty() {
        return None;
    }
    let mut msg = format!(
        "Nothing changed: the person wrote {} of this quiz's questions themselves, and this spec drops or changes {}. Put each back exactly as it is below (same text, answer, spellings and `\"source\": {{\"own\": true}}`), then call pocket_update again.\n",
        own.len(),
        if lost.len() == 1 { "one of them".to_string() } else { format!("{} of them", lost.len()) }
    );
    for (at, v) in lost {
        msg.push_str(&format!("  {at}: {v}\n"));
    }
    Some(msg)
}

/// Moves the event. An open quiz moves through the Pocket core (pa7 `set_when`), which keeps an
/// explicit close and re-plans the reminder; a draft just changes its spec. Nothing is re-checked:
/// the time isn't part of any question. The caller holds the draft's lock.
async fn move_when(state: &AppState, mut row: QuizDraftRow, when: Option<&str>) -> ToolResult {
    match State::parse(&row.state) {
        Some(State::Closed) => return fail("This quiz is closed, so its time can't move."),
        Some(State::Open) => {
            let (Some(core), Some(code)) = (pocket_core(), row.pocket_code.clone()) else {
                return fail("This kwant build has no Pocket core, so an open quiz can't move.");
            };
            if let Err(e) = core.set_when(state, &code, when).await {
                return fail(format!("The time didn't move: {e}"));
            }
        }
        _ => {}
    }
    let Some(spec) = spec_with_when(&row.spec, when) else {
        return fail("The stored quiz can't be read.");
    };
    row.spec = spec;
    row.updated_at = now_ms();
    save_and_show(state, &row).await
}

async fn dry_run(state: &AppState, row: QuizDraftRow, args: &Value) -> ToolResult {
    let _g = draft_lock(&row.id).await;
    let mut row = match fresh(state, &row).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let players = args.get("players").and_then(Value::as_u64).map(|p| p as usize).unwrap_or(qm::DRY_RUN_PLAYERS);
    match qm::dry_run(&row.spec, players) {
        Err(problems) => return fail(format!("The quiz doesn't validate yet:\n{}", problems.join("\n"))),
        Ok(d) => {
            let st = State::parse(&row.state).unwrap_or(State::Writing);
            if matches!(st, State::DryRunStopped | State::Ready) {
                row.state = if d.ok { State::Ready } else { State::DryRunStopped }.as_str().into();
            }
            row.dry_run = serde_json::to_string(&d).ok();
            row.updated_at = now_ms();
        }
    }
    save_and_show(state, &row).await
}

async fn open(state: &AppState, row: QuizDraftRow, origin: &PocketOrigin) -> ToolResult {
    // Two opens at once: the second waits here, reads the row the first saved and says it's open
    let _g = draft_lock(&row.id).await;
    let mut row = match fresh(state, &row).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    match State::parse(&row.state) {
        Some(State::Ready) => {}
        Some(State::Open) => return fail(format!("It's already open: {}.", row.pocket_code.as_deref().unwrap_or("?"))),
        _ => return fail("Only a Ready quiz opens: every answer checked against a source and a passed dry run. Fix what the card lists first."),
    }
    let Some(core) = pocket_core() else {
        return fail("This kwant build has no Pocket core yet, so the quiz can't open. It stays Ready; tell the person it's checked and waiting.");
    };
    let spec: Value = match serde_json::from_str(&row.spec) {
        Ok(v) => v,
        Err(_) => return fail("The stored quiz can't be read."),
    };
    match core.open(state, crate::kits::quiz::KIT_ID, &spec, origin).await {
        Ok(code) => {
            row.pocket_code = Some(code.clone());
            row.state = State::Open.as_str().into();
            row.updated_at = now_ms();
            let saved = if fail_this_open_save(&row.id) {
                Err("test: save refused".to_string())
            } else {
                state.db.quiz_draft_save(&row).await.map_err(|e| e.to_string())
            };
            if let Err(e) = saved {
                // A pocket the card doesn't know about would be open with nobody to close it
                tracing::warn!(error = %e, "quiz draft save failed after the pocket opened; closing it");
                return match core.close(state, &code).await {
                    Ok(()) => fail("Couldn't save the quiz, so the pocket that had just opened was closed again. The quiz is still Ready; try pocket_open again."),
                    Err(c) => fail(format!("Couldn't save the quiz after its pocket {code} opened, and closing that pocket failed too ({c}). Tell the person: pocket {code} may be open.")),
                };
            }
            let c = card_of(&row);
            (c.text(), false)
        }
        Err(e) => fail(format!("The pocket didn't open: {e}")),
    }
}

async fn close(state: &AppState, row: QuizDraftRow) -> ToolResult {
    let _g = draft_lock(&row.id).await;
    let mut row = match fresh(state, &row).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    if State::parse(&row.state) != Some(State::Open) {
        return fail("This quiz isn't open.");
    }
    let (Some(core), Some(code)) = (pocket_core(), row.pocket_code.clone()) else {
        return fail("This kwant build has no Pocket core, so there's nothing to close.");
    };
    if let Err(e) = core.close(state, &code).await {
        return fail(format!("The pocket didn't close: {e}"));
    }
    row.state = State::Closed.as_str().into();
    row.updated_at = now_ms();
    save_and_show(state, &row).await
}

// ---- the judge's model call --------------------------------------------------------------

struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The host bot's engine on its fast tier, asked once per near miss with no tools and no history.
pub struct DriverJudgeModel {
    state: AppState,
    kwant: Kwant,
}

impl DriverJudgeModel {
    pub fn new(state: AppState, kwant: Kwant) -> Self {
        Self { state, kwant }
    }
}

#[async_trait]
impl JudgeModel for DriverJudgeModel {
    async fn reply(&self, prompt: String) -> Result<String, String> {
        let state = &self.state;
        let target = self.kwant.preferred_model.clone();
        let key = super::routes::get_configured_key(&state.db, &target).await;
        let endpoint = super::routes::get_configured_endpoint(&state.db, &target).await;
        let (driver, engine) = profile_turn::driver_for(state, None, &target, key, endpoint).await?;
        let model = engine.and_then(|e| choose_model(&known_models(&state.engines, e), None, Tier::Fast)).and_then(|c| c.run_id().map(str::to_string));
        let dir = state.profiles.data_dir.join("tmp").join(format!("quiz-judge-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).map_err(|_| "no scratch folder".to_string())?;
        // Removed when the call ends, also when the judge's time limit drops this future
        let _scratch = Scratch(dir.clone());
        let input = judge_turn_input(&self.kwant, prompt, model, dir.clone());
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let out = driver.stream_turn(input, tx).await;
        drain.abort();
        out.map(|o| o.text).map_err(|e| e.to_string())
    }
}

/// The judge's one model call (pa3 qa F4): the host bot's model with a fixed one-word persona,
/// no history, no tools, read-only, no MCP servers, no skills or memory, and a scratch folder.
pub fn judge_turn_input(host: &Kwant, prompt: String, model: Option<String>, dir: std::path::PathBuf) -> TurnInput {
    TurnInput {
        thread_id: format!("quiz-judge-{}", uuid::Uuid::new_v4().simple()),
        kwant: Kwant {
            id: "quiz-judge".into(),
            name: "QuizJudge".into(),
            title: "Quiz judge".into(),
            persona_prompt: "You judge quiz answers. Reply with exactly one word: yes, no or partial.".into(),
            soul: String::new(),
            avatar_sprite: String::new(),
            temperature: 0.0,
            max_tokens: 8,
            turns_completed: 0,
            created_at: None,
            ..host.clone()
        },
        prompt,
        history: Vec::new(),
        model_override: None,
        model,
        effort_level: None,
        team_context: None,
        workspace_dir: Some(dir),
        browser_api_url: None,
        sandbox_id: None,
        teammates: Vec::new(),
        continuity: String::new(),
        skills: String::new(),
        person: Default::default(),
        store: None,
        mcp: None,
        no_tools: true,
        read_only: true,
        delivery: None,
        fresh_eyes: true,
        expects: String::new(),
    }
}

/// The judge for one open quiz pocket (pa3b, `server::quizmaster_live`): `allowance` calls, of
/// which `used` were spent before a restart. `None` when the bot is gone.
pub async fn judge_for(state: &AppState, kwant_id: &str, allowance: u32, used: u32) -> Option<Judge> {
    let kwant = state.db.get_kwant(kwant_id).await.ok().flatten()?;
    let model: Arc<dyn JudgeModel> = Arc::new(DriverJudgeModel::new(state.clone(), kwant));
    Some(Judge::new(model, Arc::new(JudgeBudget::resume(allowance, used))))
}

/// The judge's hard time limit, for the card copy ("near misses are judged in seconds").
pub const JUDGE_TIMEOUT: Duration = crate::quizmaster::judge::TIMEOUT;

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Kwant {
        serde_json::from_value(json!({
            "id": "host-bot", "name": "Quizzy", "title": "", "persona_prompt": "Be fun and use your tools.", "avatar_sprite": "",
            "driver_type": "claude", "preferred_model": "claude-opus", "temperature": 0.9, "max_tokens": 4000, "level": 1,
            "turns_completed": 12, "created_at": null
        }))
        .unwrap()
    }

    #[test]
    fn the_judge_call_has_no_tools() {
        let dir = std::env::temp_dir().join("quiz-judge-test");
        let t = judge_turn_input(&host(), "the prompt".into(), Some("fast".into()), dir.clone());
        assert!(t.no_tools, "no tools at all");
        assert!(t.read_only);
        assert!(t.mcp.is_none(), "no MCP servers");
        assert!(t.store.is_none() && t.delivery.is_none() && t.browser_api_url.is_none() && t.sandbox_id.is_none());
        assert!(t.history.is_empty() && t.teammates.is_empty() && t.skills.is_empty() && t.continuity.is_empty());
        assert!(t.team_context.is_none());
        assert_eq!(t.workspace_dir, Some(dir));
        assert_eq!((t.kwant.id.as_str(), t.kwant.max_tokens, t.kwant.temperature), ("quiz-judge", 8, 0.0));
        assert!(!t.kwant.persona_prompt.contains("tools"), "the host's persona doesn't come along");
        assert_eq!(t.kwant.preferred_model, "claude-opus", "the host's own model");
        assert_eq!(t.prompt, "the prompt");
        assert_eq!(t.model.as_deref(), Some("fast"));
    }
}
