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
            soul: String::new(),
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
            soul: String::new(),
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

#[test]
fn laya_base_url_is_normalized() {
    use kwnt::router::laya::normalize_base_url;
    assert_eq!(normalize_base_url("http://127.0.0.1:8000"), "http://127.0.0.1:8000");
    assert_eq!(normalize_base_url("http://127.0.0.1:8000/"), "http://127.0.0.1:8000");
    assert_eq!(normalize_base_url("http://127.0.0.1:8000/v1/systemone"), "http://127.0.0.1:8000");
    assert_eq!(normalize_base_url("http://127.0.0.1:8000/v1/systemone/"), "http://127.0.0.1:8000");
    assert_eq!(normalize_base_url("http://127.0.0.1:8000/v1"), "http://127.0.0.1:8000");
}

#[test]
fn laya_sidecar_only_manages_loopback_urls() {
    use kwnt::router::sidecar::local_port;
    assert_eq!(local_port("http://127.0.0.1:8000"), Some(8000));
    assert_eq!(local_port("http://localhost:9001/"), Some(9001));
    assert_eq!(local_port("http://[::1]:8000"), Some(8000));
    assert_eq!(local_port("http://localhost"), Some(80));
    assert_eq!(local_port("http://laya:8000"), None);
    assert_eq!(local_port("https://laya.example.com"), None);
}

#[test]
fn laya_sidecar_surfaces_the_failure_reason() {
    use kwnt::router::sidecar::last_error_line;
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("laya.log");
    std::fs::write(
        &log,
        "Traceback (most recent call last):\n  File \"x.py\"\nhuggingface_hub.errors.LocalEntryNotFoundError: cannot reach the Hub\n\n",
    )
    .unwrap();
    assert_eq!(
        last_error_line(&log).unwrap(),
        "huggingface_hub.errors.LocalEntryNotFoundError: cannot reach the Hub"
    );
    assert!(last_error_line(&dir.path().join("missing.log")).is_none());
}

#[tokio::test]
async fn laya_status_starts_unknown_and_updates() {
    use kwnt::router::sidecar::LayaState;
    let router = LayaRouter::new("http://127.0.0.1:1/v1/systemone".into());
    assert_eq!(router.base_url, "http://127.0.0.1:1");
    assert_eq!(router.status().state, LayaState::Unknown);
    router.set_status(LayaState::Starting, "loading", true);
    assert_eq!(router.status().state, LayaState::Starting);
    assert!(router.offline_note().contains("starting"));
}

fn agent(id: &str, name: &str, title: &str) -> Kwant {
    Kwant {
        id: id.into(),
        name: name.into(),
        title: title.into(),
        persona_prompt: format!("You are {name}. A very long persona that should not reach Laya."),
        soul: String::new(),
        avatar_sprite: "architect".into(),
        driver_type: "api".into(),
        preferred_model: "auto".into(),
        temperature: 0.7,
        max_tokens: 4096,
        level: 1,
        turns_completed: 0,
        created_at: None,
    }
}

#[test]
fn test_laya_agent_options_are_named_and_resolve_back() {
    let members = vec![
        agent("uuid-a", "Architect", "Systems Architect"),
        agent("uuid-b", "Planner", "Travel Planner"),
        agent("uuid-c", "planner", ""),
    ];
    let criteria = kwnt::router::team::kwant_choice_criteria(&members);
    let keys: Vec<&str> = criteria.keys().map(String::as_str).collect();
    assert!(keys.contains(&"Architect") && keys.contains(&"Planner") && keys.contains(&"planner #2"));
    assert_eq!(criteria["Planner"], "Travel Planner");
    assert_eq!(criteria["planner #2"], "You are planner");
    assert!(!keys.iter().any(|k| k.starts_with("uuid")));

    let pick = |choice: &str| json!({"answers": {"lead_kwant": {"choice": choice}}});
    let parse = |choice: &str| kwnt::router::parse_laya_kwant_choice(&pick(choice), &members).map(|(id, _)| id);
    assert_eq!(parse("Planner").as_deref(), Some("uuid-b"));
    assert_eq!(parse("planner #2").as_deref(), Some("uuid-c"));
    assert_eq!(parse("uuid-a").as_deref(), Some("uuid-a"));
    assert_eq!(parse("Nobody"), None);
}

#[test]
fn test_laya_team_discussion_uses_runner_up_as_second_speaker() {
    let members = vec![
        agent("a", "Architect", "Systems Architect"),
        agent("b", "Planner", "Travel Planner"),
        agent("c", "DevOps", "Systems & Security Engineer"),
    ];
    let team = Team {
        id: "t".into(),
        name: "core".into(),
        description: None,
        workspace_path: None,
        repo_url: None,
        icon: "users".into(),
        lead_kwant_id: "a".into(),
        member_kwants: "[]".into(),
        created_at: None,
    };
    let data = json!({"answers": {
        "team_decision": {"choice": "discuss"},
        "lead_kwant": {"choice": "Architect", "probabilities": {"Architect": 0.5, "Planner": 0.1, "DevOps": 0.4}}
    }});
    match kwnt::router::parse_laya_team_answers("Should we migrate to Postgres?", &data, &team, &members) {
        Some(TeamDecision::Discuss { speakers, .. }) => assert_eq!(speakers, vec!["a", "c"]),
        other => panic!("expected discussion, got {other:?}"),
    }
    let lukewarm = json!({"answers": {
        "team_decision": {"choice": "discuss", "probabilities": {"discuss": 0.72, "direct": 0.28}},
        "lead_kwant": {"choice": "Architect"}
    }});
    assert!(matches!(
        kwnt::router::parse_laya_team_answers("hi there", &lukewarm, &team, &members),
        Some(TeamDecision::Direct { kwant_id, .. }) if kwant_id == "a"
    ));
    let direct = json!({"answers": {"team_decision": {"choice": "direct"}, "lead_kwant": {"choice": "DevOps"}}});
    assert!(matches!(
        kwnt::router::parse_laya_team_answers("Who owns the deploy?", &direct, &team, &members),
        Some(TeamDecision::Direct { kwant_id, .. }) if kwant_id == "c"
    ));
}

#[test]
fn test_blend_effort_overrides_only_clear_signals() {
    use kwnt::router::fallback::blend_effort;
    assert_eq!(blend_effort("hi there", "medium"), "low");
    assert_eq!(blend_effort("Tell me a joke about Kubernetes", "medium"), "low");
    assert_eq!(blend_effort("Should we split our monolith? Compare tradeoffs", "low"), "high");
    assert_eq!(blend_effort("Postgres latency doubled, find the root cause", "medium"), "high");
    assert_eq!(blend_effort("Plan a 5-day trip to Lisbon in November", "medium"), "medium");
    assert_eq!(blend_effort("Plan a 5-day trip to Lisbon in November", "low"), "low");
    assert_eq!(blend_effort("highlight the key points of this text please", "medium"), "medium");
}

#[test]
fn test_ollama_root_accepts_any_configured_form() {
    use kwnt::drivers::ollama_root;
    for url in ["http://box:11434", "http://box:11434/", "http://box:11434/v1", "http://box:11434/v1/chat/completions"] {
        assert_eq!(ollama_root(Some(url)), "http://box:11434");
    }
}

#[tokio::test]
async fn test_ollama_not_offered_when_server_is_down() {
    std::env::remove_var("OLLAMA_MODEL");
    assert_eq!(kwnt::drivers::first_ollama_model("http://127.0.0.1:59998").await, None);
}
