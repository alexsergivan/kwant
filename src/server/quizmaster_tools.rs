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
use crate::quizmaster::source_check::{Policy, SourceResult};
use crate::quizmaster::{self as qm, preset};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
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

/// Whether pocket apps are on.
pub async fn enabled(state: &AppState) -> bool {
    matches!(state.db.get_setting(SETTING).await, Ok(Some(v)) if v == "on")
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
fn card_of(row: &QuizDraftRow) -> card::Card {
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
    if let Err(e) = state.db.quiz_draft_save(row).await {
        tracing::warn!(error = %e, "quiz draft save failed");
        return fail("Couldn't save the quiz.");
    }
    let c = card_of(row);
    let is_error = matches!(c.state, State::Writing);
    (c.text(), is_error)
}

/// Checks a spec and writes the result onto `row`, counting one writing call.
async fn check_into(row: &mut QuizDraftRow, spec_text: String, cache: &HashMap<String, SourceResult>) {
    let e = qm::evaluate(&spec_text, cache, &Policy::current()).await;
    let mut budget: Budget = parse(&row.budget);
    budget.writing_calls += 1;
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

async fn update(state: &AppState, mut row: QuizDraftRow, args: &Value) -> ToolResult {
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
    // Passed checks are reused while the question's source and answer stay the same
    let cache: HashMap<String, SourceResult> = parse::<Vec<SourceResult>>(&row.sources).into_iter().filter(|r| r.ok).map(|r| (r.key.clone(), r)).collect();
    check_into(&mut row, spec_text, &cache).await;
    save_and_show(state, &row).await
}

/// Moves the event. An open quiz moves through the Pocket core (pa7 `set_when`), which keeps an
/// explicit close and re-plans the reminder; a draft just changes its spec. Nothing is re-checked:
/// the time isn't part of any question.
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

async fn dry_run(state: &AppState, mut row: QuizDraftRow, args: &Value) -> ToolResult {
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

async fn open(state: &AppState, mut row: QuizDraftRow, origin: &PocketOrigin) -> ToolResult {
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
            row.pocket_code = Some(code);
            row.state = State::Open.as_str().into();
            row.updated_at = now_ms();
            save_and_show(state, &row).await
        }
        Err(e) => fail(format!("The pocket didn't open: {e}")),
    }
}

async fn close(state: &AppState, mut row: QuizDraftRow) -> ToolResult {
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
        let input = TurnInput {
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
                ..self.kwant.clone()
            },
            prompt,
            history: Vec::new(),
            model_override: None,
            model,
            effort_level: None,
            team_context: None,
            workspace_dir: Some(dir.clone()),
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
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let out = driver.stream_turn(input, tx).await;
        drain.abort();
        out.map(|o| o.text).map_err(|e| e.to_string())
    }
}

/// The judge for one opened quiz: pa1's adapter builds it when the pocket starts and calls
/// [`Judge::on_answer`] after each guest `answer` action, with the pocket under its lock
/// (see pa3/impl.md, "For Lin"). `None` when the bot is gone.
pub async fn judge_for(state: &AppState, kwant_id: &str, spec: &crate::kits::quiz::spec::QuizSpec) -> Option<Judge> {
    let kwant = state.db.get_kwant(kwant_id).await.ok().flatten()?;
    let model: Arc<dyn JudgeModel> = Arc::new(DriverJudgeModel::new(state.clone(), kwant));
    Some(Judge::new(model, Arc::new(JudgeBudget::new(judge_allowance(qm::text_questions(spec))))))
}

/// The judge's hard time limit, for the card copy ("near misses are judged in seconds").
pub const JUDGE_TIMEOUT: Duration = crate::quizmaster::judge::TIMEOUT;
