use kwnt::drivers::{
    events::RuntimeEvent,
    Driver, TurnInput, TurnOutput,
};
use kwnt::db::models::Kwant;
use serde_json::json;
use tokio::sync::mpsc;

#[test]
fn test_runtime_event_serialization() {
    let ev = RuntimeEvent::TextDelta("Press Start!".into());
    let serialized = serde_json::to_string(&ev).unwrap();
    assert!(serialized.contains("TextDelta"));
    assert!(serialized.contains("Press Start!"));

    let approval_ev = RuntimeEvent::ApprovalRequired {
        id: "app-1".into(),
        tool_name: "execute_shell".into(),
        details: json!({"command": "cargo build"}),
    };
    let app_str = serde_json::to_string(&approval_ev).unwrap();
    assert!(app_str.contains("ApprovalRequired"));
    assert!(app_str.contains("execute_shell"));
}

struct TestDriver;

#[async_trait::async_trait]
impl Driver for TestDriver {
    async fn stream_turn(
        &self,
        input: TurnInput,
        event_tx: mpsc::Sender<RuntimeEvent>,
    ) -> Result<TurnOutput, kwnt::drivers::DriverError> {
        event_tx
            .send(RuntimeEvent::ThinkingDelta("Calculating combo...".into()))
            .await
            .unwrap();
        event_tx
            .send(RuntimeEvent::TextDelta(format!("Player says: {}", input.prompt)))
            .await
            .unwrap();
        event_tx
            .send(RuntimeEvent::TurnComplete {
                tokens: 42,
                duration_ms: 120,
            })
            .await
            .unwrap();

        Ok(TurnOutput {
            text: format!("Player says: {}", input.prompt),
            reasoning: Some("Calculating combo...".into()),
            tokens_used: 42,
        })
    }
}

#[tokio::test]
async fn test_mock_driver_streaming() {
    let driver = TestDriver;
    let (tx, mut rx) = mpsc::channel(16);

    let dummy_kwant = Kwant {
        id: "k-1".into(),
        name: "CyberMage".into(),
        title: "Arcane".into(),
        persona_prompt: "Wizard".into(),
        soul: String::new(),
        avatar_sprite: "cyber_mage".into(),
        driver_type: "api".into(),
        preferred_model: "grok".into(),
        temperature: 0.7,
        max_tokens: 2048,
        level: 1,
        turns_completed: 0,
        created_at: None,
    };

    let input = TurnInput {
            browser_api_url: None,
        thread_id: "thread-1".into(),
        kwant: dummy_kwant,
        prompt: "Cast fireball".into(),
        history: vec![],
        model_override: None,
        effort_level: None,
        team_context: None,
        workspace_dir: None,
        sandbox_id: None,
        teammates: vec![],
    };

    let handle = tokio::spawn(async move {
        driver.stream_turn(input, tx).await.unwrap()
    });

    let mut events = vec![];
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }

    assert_eq!(events.len(), 3);
    match &events[0] {
        RuntimeEvent::ThinkingDelta(th) => assert_eq!(th, "Calculating combo..."),
        _ => panic!("Expected thinking event"),
    }
    match &events[1] {
        RuntimeEvent::TextDelta(t) => assert_eq!(t, "Player says: Cast fireball"),
        _ => panic!("Expected text event"),
    }

    let output = handle.await.unwrap();
    assert_eq!(output.tokens_used, 42);
}

#[test]
fn test_turn_input_format_system_prompt_with_and_without_team_context() {
    use kwnt::drivers::TeamContext;

    let kwant = Kwant {
        id: "k-1".into(),
        name: "Architect".into(),
        title: "Systems Architect".into(),
        persona_prompt: "You design bulletproof distributed architectures.".into(),
        soul: String::new(),
        avatar_sprite: "cyber_mage".into(),
        driver_type: "api".into(),
        preferred_model: "claude".into(),
        temperature: 0.7,
        max_tokens: 2048,
        level: 1,
        turns_completed: 0,
        created_at: None,
    };

    // 1. Without team context
    let input_solo = TurnInput {
        browser_api_url: None,
        thread_id: "thread-1".into(),
        kwant: kwant.clone(),
        prompt: "Review the system".into(),
        history: vec![],
        model_override: None,
        effort_level: None,
        team_context: None,
        workspace_dir: None,
        sandbox_id: None,
        teammates: vec![],
    };

    let solo_prompt = input_solo.format_system_prompt();
    assert!(solo_prompt.contains("Systems Architect"));
    let mut vps_input = input_solo.clone();
    vps_input.browser_api_url = Some("http://127.0.0.1:9876/api/sandbox/channel-123".into());
    assert!(vps_input.format_system_prompt().contains("http://127.0.0.1:9876/api/sandbox/channel-123/navigate"));
    assert!(!vps_input.format_system_prompt().contains("127.0.0.1:8899"));
    assert!(solo_prompt.contains("bulletproof distributed architectures"));
    assert!(!solo_prompt.contains("Active Team Alignment"));
    assert!(!solo_prompt.contains("[Teammates in this chat]"));
    assert!(!solo_prompt.contains("Your soul"));

    let mut with_soul = input_solo.clone();
    with_soul.kwant.soul = "  Calm and wry; loves a good diagram.  ".into();
    let soul_prompt = with_soul.format_system_prompt();
    assert!(soul_prompt.contains("Your soul"));
    assert!(soul_prompt.contains("Calm and wry; loves a good diagram."));
    assert!(soul_prompt.find("bulletproof").unwrap() < soul_prompt.find("Your soul").unwrap());

    let mut with_mates = input_solo.clone();
    with_mates.teammates = vec![kwnt::drivers::Teammate { name: "TechLead".into(), title: "Staff Engineer".into() }];
    let mates_prompt = with_mates.format_system_prompt();
    assert!(mates_prompt.contains("- @TechLead (Staff Engineer)"));
    assert!(mates_prompt.contains("Never contact anyone else"));

    // 2. With team context
    let input_team = TurnInput {
        browser_api_url: None,
        thread_id: "thread-team".into(),
        kwant: kwant.clone(),
        prompt: "Review the system".into(),
        history: vec![],
        model_override: None,
        effort_level: None,
        team_context: Some(TeamContext {
            id: "team-123".into(),
            name: "core-engineering".into(),
            description: Some("Core architectural, engineering, and infrastructure team".into()),
            workspace_path: None,
            repo_url: None,
        }),
        workspace_dir: None,
        sandbox_id: None,
        teammates: vec![],
    };

    let team_prompt = input_team.format_system_prompt();
    assert!(team_prompt.contains("Systems Architect"));
    assert!(team_prompt.contains("[Active Team Alignment: @core-engineering]"));
    assert!(team_prompt.contains("Team Mission: Core architectural, engineering, and infrastructure team"));
    assert!(team_prompt.contains("align your tone, recommendations, and analysis with this team's mission"));

    // 3. With team context including workspace and repo URL
    let input_workspace = TurnInput {
        browser_api_url: None,
        thread_id: "thread-workspace".into(),
        kwant: kwant.clone(),
        prompt: "Check files".into(),
        history: vec![],
        model_override: None,
        effort_level: None,
        team_context: Some(TeamContext {
            id: "team-456".into(),
            name: "app-team".into(),
            description: Some("App dev".into()),
            workspace_path: Some("/Users/dev/app".into()),
            repo_url: Some("https://github.com/org/app".into()),
        }),
        workspace_dir: Some(std::path::PathBuf::from("/Users/dev/app")),
        sandbox_id: None,
        teammates: vec![],
    };

    let ws_prompt = input_workspace.format_system_prompt();
    assert!(ws_prompt.contains("[Active Team Alignment: @app-team]"));
    assert!(ws_prompt.contains("Workspace: /Users/dev/app"));
    assert!(ws_prompt.contains("Git Repo: https://github.com/org/app"));
}

