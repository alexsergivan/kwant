//! silent-stop-asks (0.9.52): when the stuck-getting-started sweep (#1181) stops a person's reply for
//! silence, it posts a line in that chat. A person's Stop and a restart also ask in Needs you; the
//! silence stop didn't, so the person saw the stop only if they opened that chat (Rigel's
//! stuck-getting-started QA, F2). Now it asks the same way: a person's own chat gets one item that
//! points at the line; hand-offs (and routine runs and missions, whose rows say `asks = false`) don't.
//! Notes: autonomy/silent-stop-asks/impl.md.

use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        continuity::RunningTurn,
        models::NewKwant,
        spaces::SpaceId,
        threads::NewDelegation,
        waiting::{WaitingItem, STOPPED},
        Database,
    },
    router::LayaRouter,
    server::{
        lost_turns::{self, Lost},
        turns::ActiveTurn,
        AppState,
    },
    tools::ToolRunner,
};
use std::collections::HashSet;
use std::time::Duration;
use tempfile::tempdir;

const SIX_MIN: Duration = Duration::from_secs(360);
const LINE: &str = "Ivy got stuck for 6 minutes, so the work was stopped. Ask again if you still need it.";

async fn setup(tmp: &std::path::Path) -> AppState {
    let db = Database::init(tmp).await.unwrap();
    let config = Config { data_dir: tmp.join("data"), ..Config::default() };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    AppState::new(config.clone(), db, ToolRunner::new(tmp.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()))
}

async fn bot(state: &AppState, name: &str) -> String {
    let input = NewKwant {
        name: name.into(),
        title: "Tester".into(),
        persona_prompt: "p".into(),
        soul: String::new(),
        avatar_sprite: "ben".into(),
        driver_type: "api".into(),
        preferred_model: "auto".into(),
    };
    state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap().id
}

/// A running turn of `kid` in `place` (a direct chat, or a thread), silent for 90 minutes, that a
/// job still holds, with the restart record the turn writes when it starts.
async fn silent_turn(state: &AppState, turn_id: &str, kid: &str, thread_id: Option<String>, asks: Option<bool>) -> kwnt::server::turns::JobGuard {
    let since = chrono::Utc::now() - chrono::Duration::minutes(90);
    let context = thread_id.clone().unwrap_or_else(|| kid.to_string());
    let job = state.turns.start_job([context.clone(), turn_id.to_string()]);
    state.turns.set_turn(ActiveTurn {
        turn_id: turn_id.into(),
        kwant_id: kid.into(),
        context_id: context,
        thread_id: thread_id.clone(),
        state: "running".into(),
        since,
        purpose: None,
        audit: None,
    });
    state
        .db
        .add_running_turn(&RunningTurn {
            turn_id: turn_id.into(),
            kwant_id: kid.into(),
            kwant_name: "Ivy".into(),
            direct_kwant_id: Some(kid.into()),
            thread_id,
            started_at: since,
            asks,
            ..Default::default()
        })
        .await
        .unwrap();
    job
}

async fn stopped_items(state: &AppState) -> Vec<WaitingItem> {
    state.db.list_waiting(None).await.unwrap().into_iter().filter(|i| i.kind.as_deref() == Some(STOPPED)).collect()
}

/// The `turn_lost` lines in the bot's direct chat, or in a thread (`list_messages` matches either).
async fn lines(state: &AppState, kid: &str, thread_id: Option<&str>) -> Vec<kwnt::db::models::Message> {
    let msgs = state.db.list_messages(thread_id.unwrap_or(kid), 20).await.unwrap();
    msgs.into_iter().filter(|m| m.sender_role == "system" && m.meta.as_deref().is_some_and(|x| x.contains("\"turn_lost\""))).collect()
}

#[tokio::test]
async fn a_silent_stop_in_a_persons_chat_asks_once_in_needs_you() {
    let tmp = tempdir().unwrap();
    let state = setup(tmp.path()).await;
    let ivy = bot(&state, "Ivy").await;
    let job = silent_turn(&state, "t-person", &ivy, None, Some(true)).await;

    let mut cancelled = HashSet::new();
    let ended = lost_turns::sweep(&state, &mut cancelled, chrono::Utc::now(), SIX_MIN).await;
    assert_eq!(ended, vec![("t-person".to_string(), Lost::Silent)]);
    let line = lines(&state, &ivy, None).await;
    assert_eq!(line.len(), 1, "the line in the chat");
    assert_eq!(line[0].text, LINE);

    let items = stopped_items(&state).await;
    assert_eq!(items.len(), 1, "one Needs-you item: {items:?}");
    assert_eq!(items[0].kwant_id, ivy);
    assert_eq!(items[0].direct_kwant_id.as_deref(), Some(ivy.as_str()));
    assert_eq!(items[0].message_id.as_deref(), Some(line[0].id.as_str()), "it opens the line");
    assert_eq!(items[0].question, LINE, "in the line's own words: there is no Retry on it to promise");
    assert_eq!(items[0].retry, None, "the person asks again in the chat");

    // Its work never winds down: the next sweep unlists it, without a second line or a second item
    let ended = lost_turns::sweep(&state, &mut cancelled, chrono::Utc::now(), SIX_MIN).await;
    assert_eq!(ended, vec![("t-person".to_string(), Lost::Silent)]);
    assert!(!state.turns.has_turn("t-person"));
    let ended = lost_turns::sweep(&state, &mut cancelled, chrono::Utc::now(), SIX_MIN).await;
    assert!(ended.is_empty(), "{ended:?}");
    assert_eq!(lines(&state, &ivy, None).await.len(), 1);
    assert_eq!(stopped_items(&state).await.len(), 1);
    drop(job);
}

#[tokio::test]
async fn a_silent_stop_of_a_turn_that_asks_nothing_leaves_only_the_line() {
    let tmp = tempdir().unwrap();
    let state = setup(tmp.path()).await;
    let ivy = bot(&state, "Ivy").await;
    // A hand-off, a routine run or a mission task: its row says it asks nothing
    let job = silent_turn(&state, "t-handoff", &ivy, None, Some(false)).await;

    let ended = lost_turns::sweep(&state, &mut HashSet::new(), chrono::Utc::now(), SIX_MIN).await;
    assert_eq!(ended, vec![("t-handoff".to_string(), Lost::Silent)]);
    assert_eq!(lines(&state, &ivy, None).await.len(), 1, "the line still goes in its chat");
    assert!(stopped_items(&state).await.is_empty(), "its Inbox or board reports it");
    drop(job);
}

#[tokio::test]
async fn an_older_row_in_a_hand_off_thread_asks_nothing_and_one_in_a_persons_chat_asks() {
    let tmp = tempdir().unwrap();
    let state = setup(tmp.path()).await;
    let lead = bot(&state, "Lead").await;
    let ivy = bot(&state, "Ivy").await;
    let (thread, _) = state
        .db
        .create_delegation(&NewDelegation {
            lead_kwant_id: lead.clone(),
            specialist_kwant_id: ivy.clone(),
            origin_channel_id: None,
            origin_kwant_id: Some(lead.clone()),
            origin_thread_id: None,
            team_id: None,
            task: "check the numbers".into(),
            title: "Numbers".into(),
            continue_lead: false,
            depth: 1,
            follow_up_at: None,
        })
        .await
        .unwrap();
    // Rows written before 0.9.5 don't say whether they ask (`asks` NULL)
    let handoff = silent_turn(&state, "t-old-handoff", &ivy, Some(thread.id.clone()), None).await;

    let ended = lost_turns::sweep(&state, &mut HashSet::new(), chrono::Utc::now(), SIX_MIN).await;
    assert_eq!(ended, vec![("t-old-handoff".to_string(), Lost::Silent)]);
    assert_eq!(lines(&state, &ivy, Some(&thread.id)).await.len(), 1, "the line goes in the hand-off's thread");
    assert!(stopped_items(&state).await.is_empty(), "a hand-off's thread asks nothing");
    drop(handoff);

    // The same older row in the person's own chat asks, as a restart does
    let own = silent_turn(&state, "t-old-own", &ivy, None, None).await;
    let ended = lost_turns::sweep(&state, &mut HashSet::new(), chrono::Utc::now(), SIX_MIN).await;
    assert!(ended.contains(&("t-old-own".to_string(), Lost::Silent)), "{ended:?}");
    let items = stopped_items(&state).await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0].thread_id, None);
    assert_eq!(items[0].direct_kwant_id.as_deref(), Some(ivy.as_str()));
    drop(own);
}
