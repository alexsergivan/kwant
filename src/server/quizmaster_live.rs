//! pa3b: the Quizmaster, live. Wires pa3's tools and judge to the Pocket core (pa1) and its
//! schedule (pa7):
//! - [`CoreAdapter`] is the [`PocketCore`] `pocket_open`, `pocket_close` and `pocket_update`'s
//!   `when` go through: `pocket::create` (with the chat as pa7's origin), `pocket::end` and
//!   `schedule::set_when`. Registered once at start-up (`serve.rs`); every use is behind the
//!   `pocket_apps` switch (the tools refuse while it's off);
//! - [`after_answer`] is the one call `pocket::act` makes after a guest's quiz `answer`. It spawns
//!   [`judge_now`], so the guest's request never waits for a model: it reads the pocket's near
//!   misses, and only when there are some asks the pocket's one [`Judge`] (cached per code, so
//!   the allowance is per pocket). The pocket's lock is held only to read and to settle
//!   (`pocket::judge`, which saves and pushes views like a host action), never during the call;
//! - after each judging round the calls used go into the draft's budget (`judge_calls`), and a
//!   judge made after a restart starts from them (pa3 qa F3).

use super::quizmaster_tools::{self as tools, PocketCore, PocketOrigin};
use super::AppState;
use crate::db::pocket::OPEN;
use crate::kits::quiz::engine::JudgeVerdict;
use crate::kits::quiz::KIT_ID;
use crate::kits::{ActionError, ActionOutcome};
use crate::pocket::quiz_kit::QuizPocket;
use crate::pocket::{self, schedule, CreateRequest, PocketError};
use crate::quizmaster::budget::{judge_allowance, Budget};
use crate::quizmaster::judge::{judge_body, Judge, JudgeBudget, JudgeModel, JudgeTarget, Outcome, Pending};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// pa1 + pa7 behind pa3's [`PocketCore`].
pub struct CoreAdapter;

/// The core's refusal as the plain sentence the bot reads (pa7's `bad_origin`, `ended`,
/// `bad_times`, the limits: each already has one).
fn words(e: PocketError) -> String {
    e.message
}

#[async_trait]
impl PocketCore for CoreAdapter {
    async fn open(&self, state: &AppState, kit: &str, spec: &serde_json::Value, origin: &PocketOrigin) -> Result<String, String> {
        let req = CreateRequest {
            kit: kit.to_string(),
            spec: spec.clone(),
            owner_kwant_id: Some(origin.kwant_id.clone()),
            opens_at: None,
            closes_at: None,
            origin: Some(schedule::Origin { kwant_id: origin.kwant_id.clone(), channel_id: origin.channel_id.clone(), thread_id: origin.thread_id.clone() }),
        };
        // The host key isn't kept: the person runs the show from their own session (pa1b links)
        let (snap, _host_key) = pocket::create(state, req).await.map_err(words)?;
        Ok(snap.row.code.clone())
    }

    async fn close(&self, state: &AppState, code: &str) -> Result<(), String> {
        forget(code);
        pocket::end(state, code).await.map(|_| ()).map_err(words)
    }

    async fn set_when(&self, state: &AppState, code: &str, when: Option<&str>) -> Result<(), String> {
        let when = match when {
            None => None,
            Some(t) => Some(chrono::DateTime::parse_from_rfc3339(t.trim()).map_err(|_| "the time must be RFC 3339 with its offset, like 2026-11-27T21:00:00+00:00".to_string())?),
        };
        schedule::set_when(state, code, when).await.map(|_| ()).map_err(words)
    }
}

// ---- judging on arrival ------------------------------------------------------------------

/// One judge per open quiz pocket, by code.
fn judges() -> &'static Mutex<HashMap<String, Arc<Judge>>> {
    static JUDGES: OnceLock<Mutex<HashMap<String, Arc<Judge>>>> = OnceLock::new();
    JUDGES.get_or_init(Default::default)
}

fn forget(code: &str) {
    judges().lock().unwrap_or_else(|e| e.into_inner()).remove(code);
}

static TEST_MODEL: Mutex<Option<Arc<dyn JudgeModel>>> = Mutex::new(None);

/// Tests: judges made from now on ask this model instead of the host bot's engine.
#[cfg(feature = "test-seams")]
#[doc(hidden)]
pub fn use_judge_model_for_tests(model: Arc<dyn JudgeModel>) {
    *TEST_MODEL.lock().unwrap_or_else(|e| e.into_inner()) = Some(model);
}

/// Tests: drops this pocket's cached judge, as a restart would.
#[cfg(feature = "test-seams")]
#[doc(hidden)]
pub fn forget_judge_for_tests(code: &str) {
    forget(code);
}

/// The hook `pocket::act` calls after a guest's `answer` (with the pocket still locked): returns
/// at once; the judging runs on its own task.
pub fn after_answer(state: &AppState, code: &str, kit: &str) {
    if kit != KIT_ID {
        return;
    }
    let (state, code) = (state.clone(), code.to_string());
    tokio::spawn(async move {
        judge_now(&state, &code).await;
    });
}

/// Judges the pocket's near misses now; what [`after_answer`] runs. Nothing happens while the
/// switch is off, when nothing is pending, or for a pocket no quiz draft opened.
pub async fn judge_now(state: &AppState, code: &str) -> Vec<Outcome> {
    if !pocket::enabled(&state.db).await {
        return Vec::new();
    }
    let target = PocketTarget { state, code };
    if !is_open(state, code).await {
        forget(code);
        return Vec::new();
    }
    // pa11 qa F4: a class quiz says "Nothing is shared", so a guest's guess never goes to a model
    // (the host's may be a hosted one). Its near misses count 0 until the host settles them.
    if matches!(pocket::snapshot(state, code).await, Ok(Some(s)) if crate::pocket::quiz_kit::is_class(&s.spec)) {
        return Vec::new();
    }
    if target.pending().await.is_empty() {
        return Vec::new();
    }
    let Some(judge) = judge_of(state, code).await else { return Vec::new() };
    let out = judge.on_answer(&target).await;
    // BudgetOut makes no call; anything else used one
    if out.iter().any(|o| !matches!(o, Outcome::BudgetOut { .. })) {
        save_used(state, code, judge.budget()).await;
    }
    out
}

async fn is_open(state: &AppState, code: &str) -> bool {
    matches!(pocket::snapshot(state, code).await, Ok(Some(s)) if s.row.state == OPEN && s.row.kit == KIT_ID)
}

/// The pocket's judge: cached, else made from its draft (allowance and calls already used).
async fn judge_of(state: &AppState, code: &str) -> Option<Arc<Judge>> {
    if let Some(j) = judges().lock().unwrap_or_else(|e| e.into_inner()).get(code) {
        return Some(j.clone());
    }
    let draft = state.db.quiz_draft_by_pocket(code).await.ok().flatten()?;
    let budget: Budget = serde_json::from_str(&draft.budget).unwrap_or_default();
    let allowance = if budget.judge_allowance > 0 { budget.judge_allowance } else { allowance_of(&draft.spec) };
    let test_model = TEST_MODEL.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let judge = match test_model {
        Some(model) => Judge::new(model, Arc::new(JudgeBudget::resume(allowance, budget.judge_calls))),
        None => tools::judge_for(state, &draft.kwant_id, allowance, budget.judge_calls).await?,
    };
    // Two first answers at once: the first judge in wins, both use it
    Some(judges().lock().unwrap_or_else(|e| e.into_inner()).entry(code.to_string()).or_insert_with(|| Arc::new(judge)).clone())
}

fn allowance_of(spec: &str) -> u32 {
    crate::kits::quiz::validate::parse(spec).map_or(judge_allowance(0), |s| judge_allowance(crate::quizmaster::text_questions(&s)))
}

/// Writes the calls used into the draft's budget, under the draft's lock (pa3 qa F2/F3).
async fn save_used(state: &AppState, code: &str, budget: &JudgeBudget) {
    let Ok(Some(draft)) = state.db.quiz_draft_by_pocket(code).await else { return };
    let _g = tools::draft_lock(&draft.id).await;
    if let Err(e) = state.db.quiz_draft_set_judge_calls(&draft.id, budget.used()).await {
        tracing::warn!(error = %e, "couldn't save the quiz judge's call count");
    }
}

/// A live pocket as the judge sees it.
struct PocketTarget<'a> {
    state: &'a AppState,
    code: &'a str,
}

#[async_trait]
impl JudgeTarget for PocketTarget<'_> {
    async fn pending(&self) -> Vec<Pending> {
        match pocket::snapshot(self.state, self.code).await {
            Ok(Some(s)) if s.row.state == OPEN && s.row.kit == KIT_ID => serde_json::from_value(QuizPocket::pending(&s.spec, &s.kit_state)).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    async fn settle(&self, q: usize, player: &str, verdict: JudgeVerdict) -> Result<ActionOutcome, ActionError> {
        pocket::judge(self.state, self.code, &judge_body(q, player, verdict)).await.map(|()| ActionOutcome::Applied).map_err(|e| ActionError::WrongPhase(e.message))
    }
}
