use kwnt::router::{parse_laya_response, LayaRouter};
use serde_json::json;

#[tokio::test]
async fn test_offline_fallback_routing() {
    // Port 59999 is intentionally offline
    let router = LayaRouter::new("http://127.0.0.1:59999".into());
    let decision = router
        .route_prompt("Write a quick python script for sorting", "code")
        .await
        .unwrap();

    assert!(decision.is_fallback);
    assert!(!decision.selected_model.is_empty());
    // For coding, fallback routes to codex or grok
    assert!(decision.selected_model == "codex" || decision.selected_model == "grok");
    assert!(!decision.effort_level.is_empty());
}

#[tokio::test]
async fn test_fallback_heuristic_variety() {
    let router = LayaRouter::new("http://127.0.0.1:59999".into());

    let arch_decision = router
        .route_prompt("Analyze multi-region architecture tradeoffs and write a detailed design doc", "architecture")
        .await
        .unwrap();
    assert_eq!(arch_decision.selected_model, "claude");
    assert_eq!(arch_decision.effort_level, "high");

    let low_decision = router
        .route_prompt("hi there", "greeting")
        .await
        .unwrap();
    assert_eq!(low_decision.effort_level, "low");

    let privacy_decision = router
        .route_prompt("Run private local query without sending to cloud, confidential data", "internal")
        .await
        .unwrap();
    assert_eq!(privacy_decision.selected_model, "local_ollama");
}

#[tokio::test]
async fn test_route_prompt_constrained_by_available_models() {
    let router = LayaRouter::new("http://127.0.0.1:59999".into());

    // Host only has grok available
    let available = vec![kwnt::drivers::ModelDescriptor {
        id: "grok".into(),
        display_name: "xAI Grok".into(),
        capability_desc: "fast technical reasoning".into(),
        is_cli: true,
    }];

    // Prompt contains architecture keywords, but Claude is NOT available
    let decision = router
        .route_prompt_with_models(
            "Analyze multi-region architecture tradeoffs",
            "architecture",
            &available,
        )
        .await
        .unwrap();

    // Must gracefully fall back to the available Grok model
    assert_eq!(decision.selected_model, "grok");
    assert_eq!(decision.effort_level, "high");
}

#[test]
fn test_parse_laya_success_response() {
    let mock_json = json!({
        "answers": {
            "model_selection": {
                "choice": "grok",
                "confidence": 0.94
            },
            "effort_level": {
                "choice": "high"
            }
        },
        "routing": {
            "model": "english"
        }
    });

    let decision = parse_laya_response(&mock_json).unwrap();
    assert_eq!(decision.selected_model, "grok");
    assert_eq!(decision.effort_level, "high");
    assert_eq!(decision.confidence, 0.94);
    assert!(!decision.is_fallback);
}
