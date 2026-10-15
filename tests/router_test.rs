use kwnt::db::models::{Kwant, Team};
use kwnt::router::{parse_laya_response, parse_mentions, LayaRouter, TagMention, TeamDecision};
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

#[test]
fn test_parse_mentions_kwant_and_team() {
    let input = "Hey @Architect and @core-engineering, please review this PR.";
    let mentions = parse_mentions(input);
    assert_eq!(mentions.len(), 2);
    assert!(matches!(&mentions[0], TagMention::Kwant(name) if name == "Architect"));
    assert!(matches!(&mentions[1], TagMention::Team(name) if name == "core-engineering"));
}

#[tokio::test]
async fn test_team_orchestration_heuristic_fallback() {
    let router = LayaRouter::new("http://127.0.0.1:9999/unreachable".into());
    let team = Team {
        id: "team-1".into(),
        name: "core-engineering".into(),
        description: Some("Architecture & DevOps".into()),
        workspace_path: None,
        repo_url: None,
        icon: "users".into(),
        lead_kwant_id: "kwant-1".into(),
        member_kwants: "[\"kwant-1\", \"kwant-2\"]".into(),
        created_at: None,
    };
    let members = vec![
        Kwant {
            id: "kwant-1".into(),
            name: "Architect".into(),
            title: "Systems Architect".into(),
            persona_prompt: "Architecture".into(),
            avatar_sprite: "architect".into(),
            driver_type: "api".into(),
            preferred_model: "auto".into(),
            temperature: 0.7,
            max_tokens: 4096,
            level: 1,
            turns_completed: 0,
            created_at: None,
        },
        Kwant {
            id: "kwant-2".into(),
            name: "DevOps".into(),
            title: "DevOps Engineer".into(),
            persona_prompt: "DevOps".into(),
            avatar_sprite: "devops".into(),
            driver_type: "api".into(),
            preferred_model: "auto".into(),
            temperature: 0.7,
            max_tokens: 4096,
            level: 1,
            turns_completed: 0,
            created_at: None,
        },
    ];

    // For complex question, fallback should trigger multi-turn discussion
    let decision = router.route_team_query("Architectural overhaul and cluster migration tradeoff analysis", &team, &members).await;
    match decision {
        TeamDecision::Discuss { speakers, turns, .. } => {
            assert!(turns >= 2);
            assert_eq!(speakers.len(), turns);
        }
        _ => panic!("Expected discuss decision"),
    }
}


#[test]
fn pinned_preferred_model_overrides_laya_but_keeps_effort() {
    use kwnt::router::{apply_preferred_model, RouteDecision};
    let laya = || RouteDecision {
        selected_model: "claude".into(),
        effort_level: "high".into(),
        confidence: 0.7,
        provider_endpoint: None,
        is_fallback: false,
        reasoning: "laya".into(),
    };

    let pinned = apply_preferred_model(laya(), "grok");
    assert_eq!(pinned.selected_model, "grok");
    assert_eq!(pinned.effort_level, "high");
    assert!(pinned.reasoning.contains("pinned"));

    for unpinned in ["auto", "AUTO", "", "  "] {
        assert_eq!(apply_preferred_model(laya(), unpinned).selected_model, "claude");
    }
}
