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
        thread_id: "thread-1".into(),
        kwant: dummy_kwant,
        prompt: "Cast fireball".into(),
        history: vec![],
        model_override: None,
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
