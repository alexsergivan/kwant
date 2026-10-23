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
        space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
        id: "team-1".into(),
        name: "core-engineering".into(),
        description: Some("Architecture & DevOps".into()),
        workspace_path: None,
        repo_url: None,
        icon: "users".into(),
        lead_kwant_id: "kwant-1".into(),
        member_kwants: "[\"kwant-1\", \"kwant-2\"]".into(),
        created_at: None,
        instructions: None,
    };
    let members = vec![
        Kwant {
            space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
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
            space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
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
        tier: None,
        model: None,
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
fn preferred_model_pin_forms() {
    use kwnt::router::{parse_pin, Pin};
    let pin = |f: Option<&str>, m: Option<&str>| Pin { family: f.map(Into::into), model: m.map(Into::into) };
    // auto
    for auto in ["auto", "Auto", "", " "] {
        assert_eq!(parse_pin(auto), pin(None, None));
    }
    // <engine>
    assert_eq!(parse_pin("claude"), pin(Some("claude"), None));
    assert_eq!(parse_pin("codex"), pin(Some("codex"), None));
    assert_eq!(parse_pin("ollama"), pin(Some("local_ollama"), None));
    assert_eq!(parse_pin("local_ollama"), pin(Some("local_ollama"), None), "old value");
    // <engine>:<model> and <engine>/<model>
    assert_eq!(parse_pin("claude:sonnet"), pin(Some("claude"), Some("sonnet")));
    assert_eq!(parse_pin("claude/sonnet"), pin(Some("claude"), Some("sonnet")));
    assert_eq!(parse_pin("codex/gpt-6.1-sol"), pin(Some("codex"), Some("gpt-6.1-sol")));
    assert_eq!(parse_pin("grok:auto"), pin(Some("grok"), None));
    assert_eq!(parse_pin("ollama/llama3.2:latest"), pin(Some("local_ollama"), Some("llama3.2:latest")));
    assert_eq!(parse_pin("ollama:llama3.2:latest"), pin(Some("local_ollama"), Some("llama3.2:latest")));
    // Old model-id pins keep working
    assert_eq!(parse_pin("claude-opus-5-5"), pin(Some("claude"), Some("claude-opus-5-5")));
    // Custom API: the model id stays whole, even an OpenRouter one with its own slash
    assert_eq!(parse_pin("custom"), pin(Some("custom"), None));
    assert_eq!(parse_pin("custom:gpt-4.1"), pin(Some("custom"), Some("gpt-4.1")));
    assert_eq!(parse_pin("custom:openai/gpt-4o"), pin(Some("custom"), Some("openai/gpt-4o")));
    assert_eq!(parse_pin("custom/claude-sonnet-4"), pin(Some("custom"), Some("claude-sonnet-4")));
    // A Claude model on a Custom API account is still Custom API, never the Claude engine
    assert_eq!(
        kwnt::profiles::Engine::from_model("custom:claude-sonnet-4"),
        Some(kwnt::profiles::Engine::Custom)
    );
}

#[test]
fn custom_api_never_runs_on_the_hosts_keys_or_clis() {
    use kwnt::drivers::engines::{system_route, EngineRegistry, NO_CUSTOM_ACCOUNT};
    let engines = EngineRegistry::default();
    let route = system_route(&engines, "custom", Some("sk-host-setting-key".into()), |_| Some("/usr/bin/claude".into()));
    assert_eq!(route, Err(NO_CUSTOM_ACCOUNT.to_string()));
}

#[test]
fn pins_set_tier_and_model_on_the_route() {
    use kwnt::drivers::engines::Tier;
    use kwnt::router::{apply_preferred_model, RouteDecision};
    let laya = |effort: &str| RouteDecision {
        selected_model: "codex".into(),
        effort_level: effort.into(),
        confidence: 0.7,
        provider_endpoint: None,
        is_fallback: false,
        tier: None,
        model: None,
        reasoning: "laya".into(),
    };
    let auto = apply_preferred_model(laya("low"), "auto");
    assert_eq!((auto.selected_model.as_str(), auto.tier, auto.model), ("codex", Some(Tier::Fast), None));

    let engine = apply_preferred_model(laya("high"), "claude");
    assert_eq!((engine.selected_model.as_str(), engine.tier, engine.model), ("claude", Some(Tier::Deep), None));

    let model = apply_preferred_model(laya("low"), "claude:sonnet");
    assert_eq!((model.selected_model.as_str(), model.model.as_deref()), ("claude", Some("sonnet")));
    assert!(model.reasoning.contains("Model pinned"), "{}", model.reasoning);
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
        space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
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
        space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
        id: "t".into(),
        name: "core".into(),
        description: None,
        workspace_path: None,
        repo_url: None,
        icon: "users".into(),
        lead_kwant_id: "a".into(),
        member_kwants: "[]".into(),
        created_at: None,
        instructions: None,
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
    assert_eq!(blend_effort("Should we split our monolith? Compare tradeoffs", "medium"), "high");
    // Laya's "low" stands: a keyword alone no longer raises it (laya-eval a01, a03)
    assert_eq!(blend_effort("Should we split our monolith? Compare tradeoffs", "low"), "low");
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

/// Alex's finding 4: in a Go trading repo, "Create a new test simple strategy" went to Frontend
/// because Laya only compares titles. A code request goes to the role it belongs to; anything that
/// matches no role, or several, goes to the lead; the reason says which.
fn trading_team() -> (Team, Vec<Kwant>) {
    let members = vec![
        agent("lead", "Ada", "Tech Lead"),
        agent("fe", "Pixel", "Frontend Engineer"),
        agent("be", "Lin", "Backend Engineer (Go)"),
        agent("sre", "Ops", "SRE"),
        agent("qa", "Rigel", "QA Engineer"),
    ];
    let team = Team {
        space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
        id: "t".into(),
        name: "trading".into(),
        description: Some("Go trading bot".into()),
        workspace_path: None,
        repo_url: None,
        icon: "users".into(),
        lead_kwant_id: "lead".into(),
        member_kwants: "[]".into(),
        created_at: None,
        instructions: None,
    };
    (team, members)
}

fn direct(d: TeamDecision) -> (String, String, String, Vec<String>) {
    match d {
        TeamDecision::Direct { kwant_id, reason, basis, matched_terms } => (kwant_id, reason, basis, matched_terms),
        other => panic!("expected a direct answer, got {other:?}"),
    }
}

#[test]
fn code_requests_go_to_the_matching_role_and_the_rest_to_the_lead() {
    let (team, members) = trading_team();
    let route = |p: &str| direct(kwnt::router::heuristic_team_decision(p, &team, &members));

    let (id, reason, basis, terms) = route("Create a new test simple strategy");
    assert_eq!((id.as_str(), basis.as_str()), ("be", "keywords"), "{reason}");
    assert!(terms.contains(&"strategy".to_string()), "{terms:?}");
    assert!(reason.starts_with("Lin (Backend Engineer (Go)) does this kind of work"), "{reason}");

    for (prompt, who) in [
        ("Fix the flaky e2e tests", "qa"),
        ("Check why the deploy pipeline fails", "sre"),
        ("Make the settings button blue on the dashboard page", "fe"),
        ("Add a backtest for the moving average strategy", "be"),
        ("Rigel, can you look at this?", "qa"),
    ] {
        assert_eq!(route(prompt).0, who, "{prompt}");
    }

    let (id, reason, basis, _) = route("What's the capital of Peru?");
    assert_eq!((id.as_str(), basis.as_str()), ("lead", "lead"));
    assert_eq!(reason, "No one's role matched this, so it went to the team lead, Ada (Tech Lead)");

    let (id, reason, basis, terms) = route("Add an API endpoint and a settings page that shows it");
    assert_eq!((id.as_str(), basis.as_str()), ("lead", "several"), "{reason}");
    assert!(reason.starts_with("This touches several roles (") && reason.contains("Lin (Backend Engineer (Go))"), "{reason}");
    assert!(reason.contains("Pixel (Frontend Engineer)") && reason.ends_with("Ada (Tech Lead), to answer or hand parts on"), "{reason}");
    assert!(terms.contains(&"endpoint".to_string()) && terms.contains(&"page".to_string()), "{terms:?}");

    // Also when the lead is one of the roles
    let mut led_by_backend = team.clone();
    led_by_backend.lead_kwant_id = "be".into();
    let (id, reason, basis, _) = direct(kwnt::router::heuristic_team_decision("Add an API endpoint and a settings page that shows it", &led_by_backend, &members));
    assert_eq!((id.as_str(), basis.as_str()), ("be", "several"), "{reason}");
    assert!(reason.ends_with("so it went to the team lead, Lin (Backend Engineer (Go)), to answer or hand parts on"), "{reason}");

    // Discussion words still start a discussion, but whole words only and not "strategy"
    assert!(matches!(
        kwnt::router::heuristic_team_decision("Should we migrate to Postgres? Compare the tradeoffs", &team, &members),
        TeamDecision::Discuss { .. }
    ));
    assert_eq!(route("Explain how orders are matched").2, "keywords", "'explain' is not 'plan'");
}

#[test]
fn a_weak_laya_pick_no_longer_overrides_the_role() {
    let (team, members) = trading_team();
    // What Alex saw: Laya sure enough to discuss, and its lead pick is Frontend
    let laya = json!({"answers": {
        "team_decision": {"choice": "discuss", "probabilities": {"discuss": 0.86, "direct": 0.14}},
        "lead_kwant": {"choice": "Pixel", "probabilities": {"Pixel": 0.4, "Lin": 0.2, "Ada": 0.2, "Ops": 0.1, "Rigel": 0.1}}
    }});
    let d = kwnt::router::parse_laya_team_answers("Create a new test simple strategy", &laya, &team, &members).unwrap();
    let (id, reason, basis, _) = direct(d);
    assert_eq!((id.as_str(), basis.as_str()), ("be", "keywords"));
    assert!(reason.contains("Laya guessed Pixel from titles alone"), "{reason}");

    // Laya and the words agree
    let agree = json!({"answers": {"team_decision": {"choice": "direct"}, "lead_kwant": {"choice": "Lin"}}});
    let (id, _, basis, _) = direct(kwnt::router::parse_laya_team_answers("Create a new test simple strategy", &agree, &team, &members).unwrap());
    assert_eq!((id.as_str(), basis.as_str()), ("be", "skills"));

    // No words match: a clear Laya pick stands, a near-tie goes to the lead
    let clear = json!({"answers": {"team_decision": {"choice": "direct"},
        "lead_kwant": {"choice": "Ops", "probabilities": {"Ops": 0.7, "Lin": 0.1, "Ada": 0.1, "Pixel": 0.05, "Rigel": 0.05}}}});
    let (id, reason, basis, _) = direct(kwnt::router::parse_laya_team_answers("Is the box healthy?", &clear, &team, &members).unwrap());
    assert_eq!((id.as_str(), basis.as_str()), ("sre", "skills"), "{reason}");
    let close = json!({"answers": {"team_decision": {"choice": "direct"},
        "lead_kwant": {"choice": "Pixel", "probabilities": {"Pixel": 0.3, "Lin": 0.25, "Ada": 0.25, "Ops": 0.1, "Rigel": 0.1}}}});
    let (id, _, basis, _) = direct(kwnt::router::parse_laya_team_answers("Is the box healthy?", &close, &team, &members).unwrap());
    assert_eq!((id.as_str(), basis.as_str()), ("lead", "lead"));

    // A real multi-role question can still be a discussion
    let plan = json!({"answers": {"team_decision": {"choice": "discuss", "probabilities": {"discuss": 0.9, "direct": 0.1}},
        "lead_kwant": {"choice": "Ada"}}});
    assert!(matches!(
        kwnt::router::parse_laya_team_answers("Plan the migration of orders to Postgres", &plan, &team, &members),
        Some(TeamDecision::Discuss { .. })
    ));
}

#[tokio::test]
async fn channel_routing_reports_its_basis() {
    let (_, members) = trading_team();
    let laya = kwnt::router::LayaRouter::new("http://127.0.0.1:9".into());
    let r = laya.route_channel_query_detailed("Create a new test simple strategy", "desk", "lead", &members).await;
    assert_eq!((r.kwant_id.as_str(), r.basis.as_str(), r.fallback), ("be", "keywords", true));
    assert!(r.matched_terms.contains(&"strategy".to_string()));
    let r = laya.route_channel_query_detailed("hello", "desk", "lead", &members).await;
    assert_eq!((r.kwant_id.as_str(), r.basis.as_str()), ("lead", "lead"));
    assert!(r.reason.starts_with("No one's role matched this, so it went to the channel lead, Ada"), "{}", r.reason);
    let r = laya.route_channel_query_detailed("hello", "desk", "lead", &members[1..2]).await;
    assert_eq!((r.kwant_id.as_str(), r.basis.as_str()), ("fe", "only"));
}

// Cases below come from the Laya routing eval (shared/ship-software-safely/laya-eval/eval.json); the
// id in each message is the eval prompt's id.

const EVAL_MODELS: [&str; 4] = ["claude", "grok", "codex", "local_ollama"];

fn offline_model(prompt: &str) -> String {
    let models: Vec<String> = EVAL_MODELS.iter().map(|m| m.to_string()).collect();
    kwnt::router::fallback::heuristic_fallback_with_models(prompt, "chat", &models).selected_model
}

#[test]
fn offline_keywords_match_whole_words_only() {
    // "local" inside another word, or a local business, is not a request to stay on this machine
    for (id, prompt) in [
        ("m17", "Why does my React app fail to reach localhost:3000 from inside the Docker container?"),
        ("m18", "How do I set the locale to de_DE in a Python script for number formatting?"),
        ("m25", "Is it safe to store JWTs in localStorage?"),
        ("a07", "My local coffee shop wants a tagline for its new menu. Give me 5 ideas."),
    ] {
        assert_ne!(offline_model(prompt), "local_ollama", "{id}: {prompt}");
    }
    // "script" inside "description" is not code
    assert_eq!(offline_model("Write a punchy product description for our handmade soy candle shop"), "claude", "m11");
    assert_eq!(offline_model("I don't trust this vendor, draft a polite email asking for references"), "claude", "'rust' in 'trust'");

    // Explicit requests still stay local
    for (id, prompt) in [
        ("m14", "Summarize my private medical lab results: total cholesterol 240, LDL 160, HDL 38"),
        ("m15", "This is confidential: find duplicate entries in my password manager export"),
        ("m16", "I'm offline on a plane. Rename these photos by date taken"),
        ("-", "Use a local model for this, it's private data"),
        ("-", "Run this locally with ollama"),
    ] {
        assert_eq!(offline_model(prompt), "local_ollama", "{id}: {prompt}");
    }
    // And whole words still route as before, plurals included
    assert_eq!(offline_model("How do I set the locale to de_DE in a Python script for number formatting?"), "codex", "m18");
    assert_eq!(offline_model("Write two Python scripts that rename every .jpeg file to .jpg"), "codex", "m21");
    assert_eq!(offline_model("Design a multi-tenant billing system and explain the tradeoffs"), "claude", "m10");
}

#[test]
fn negated_and_idiomatic_keywords_dont_make_offline_effort_high() {
    use kwnt::router::fallback::heuristic_effort;
    assert_eq!(heuristic_effort("Please don't refactor anything, just tell me what time zone Paris is in", "chat"), "low", "a01");
    assert_ne!(heuristic_effort("Write a friendly reminder text for the security guards about the Friday night shift", "chat"), "high", "a02");
    assert_eq!(heuristic_effort("Explain what a 'deep copy' is in Python in one line", "chat"), "low", "a03");
    // Real signals still count
    for (id, prompt) in [
        ("m08", "Refactor the auth module across the 12 files in src/auth to use one Session trait and update every call site"),
        ("m09", "Our checkout service times out under load but only on Tuesdays. Help me find the root cause; CPU and DB look normal."),
        ("m10", "Design a multi-tenant billing system with usage-based pricing and explain the tradeoffs"),
        ("-", "Do a security review of the login flow"),
        ("-", "Plan the migration of our orders table"),
    ] {
        assert_eq!(heuristic_effort(prompt, "chat"), "high", "{id}: {prompt}");
    }
    // A Chinese paragraph is many words, not one trivial word
    assert_ne!(heuristic_effort("请帮我分析这个微服务架构的取舍，并给出迁移方案", "chat"), "low", "m24");
    assert_eq!(heuristic_effort("你好", "chat"), "low");
}

#[test]
fn keywords_raise_layas_medium_but_not_its_low() {
    use kwnt::router::fallback::blend_effort;
    // Laya said medium; a clear keyword makes these high (m08-m10)
    assert_eq!(blend_effort("Refactor the auth module across the 12 files in src/auth to use one Session trait and update every call site", "medium"), "high", "m08");
    assert_eq!(blend_effort("Our checkout service times out under load but only on Tuesdays. Help me find the root cause; CPU and DB look normal.", "medium"), "high", "m09");
    assert_eq!(blend_effort("Design a multi-tenant billing system with usage-based pricing and explain the tradeoffs", "medium"), "high", "m10");
    // Laya said low; a keyword that is only mentioned doesn't overrule it (a01, a03)
    assert_eq!(blend_effort("Please don't refactor anything, just tell me what time zone Paris is in", "low"), "low", "a01");
    assert_eq!(blend_effort("Explain what a 'deep copy' is in Python in one line", "low"), "low", "a03");
    // Not a deep signal at all (a02)
    assert_eq!(blend_effort("Write a friendly reminder text for the security guards about the Friday night shift", "medium"), "medium", "a02");
}

#[test]
fn laya_model_payload_keeps_its_measured_key_order() {
    // Laya's picks depend on the order it sees questions and options in. Sorted keys (effort_level
    // first, options alphabetical) scored 30/33 on the routing eval; written order scored 24-25/33.
    // If this fails because serde_json's preserve_order got enabled, re-run the eval before accepting it.
    let mut criteria = serde_json::Map::new();
    criteria.insert("grok".into(), json!("g"));
    criteria.insert("claude".into(), json!("c"));
    criteria.insert("local_ollama".into(), json!("l"));
    criteria.insert("codex".into(), json!("x"));
    let payload = kwnt::router::laya::model_payload("hi", "chat", criteria);
    assert_eq!(
        serde_json::to_string(&payload).unwrap(),
        concat!(
            r#"{"questions":{"effort_level":{"criteria":{"high":"deep architecture, difficult bug root cause analysis, complex refactoring","#,
            r#""low":"quick answers, greetings, short definitions, simple edits","#,
            r#""medium":"standard coding, explanations, moderate feature implementation"},"#,
            r#""instructions":"What reasoning effort / compute depth is needed for this task?","type":"choice"},"#,
            r#""model_selection":{"criteria":{"claude":"c","codex":"x","grok":"g","local_ollama":"l"},"#,
            r#""instructions":"Which available model is best suited for this task?","type":"choice"}},"#,
            r#""state":{"context":"chat","prompt":"hi"}}"#,
        )
    );
}

/// The 5-bot team from the routing eval, each bot with its title and one-line persona.
fn eval_team() -> Vec<Kwant> {
    let bot = |name: &str, title: &str, persona: &str| {
        let mut k = agent(&name.to_lowercase(), name, title);
        k.persona_prompt = persona.into();
        k
    };
    vec![
        bot("Dev", "Full-stack Software Engineer", "You are a senior full-stack engineer who writes and reviews code."),
        bot("Mia", "Marketing Manager", "You write marketing copy, campaigns and newsletters."),
        bot("Remy", "Research Analyst", "You research markets and competitors and analyze data."),
        bot("Dana", "UI/UX Designer", "You design interfaces, logos and brand visuals."),
        bot("Quinn", "QA Engineer", "You test software and write test plans."),
    ]
}

#[test]
fn marketing_research_design_and_generic_engineers_have_roles() {
    let members = eval_team();
    let pick = |prompt: &str| kwnt::router::team::pick_member(prompt, "dev", &members, None, "channel lead").unwrap().kwant.name.clone();
    for (id, prompt, who) in [
        ("b03", "Write 5 tweets announcing our launch", "Mia"),
        ("b13", "Improve the SEO of our landing page copy", "Mia"),
        ("x02", "Write a test plan for our ad campaign A/B test", "Mia"),
        ("b04", "Find three competitors and compare their pricing", "Remy"),
        ("b07", "What's the market size for language-learning apps?", "Remy"),
        ("b22", "Analyze our churn data and suggest why users leave", "Remy"),
        ("b15", "Is the new checkout flow confusing? Do a usability review", "Dana"),
        ("b21", "Make the onboarding screens prettier", "Dana"),
        ("b06", "Design a new logo concept for the app", "Dana"),
        ("b23", "Write a Python script to scrape competitor prices", "Dev"),
        ("b02", "Fix the 500 error in the /orders endpoint", "Dev"),
        ("b05", "Write test cases for the password reset flow", "Quinn"),
    ] {
        assert_eq!(pick(prompt), who, "{id}: {prompt}");
    }
    // A specific title wins over the generic "engineer": Quinn is QA, not backend
    assert_eq!(pick("Add a database index to speed up the search queries"), "Dev", "b09");
}

#[test]
fn a_clear_laya_pick_beats_words_only_in_a_persona() {
    let members = eval_team();
    let laya_says = |name: &str| {
        let mut probs = serde_json::Map::new();
        for m in &members {
            probs.insert(m.name.clone(), json!(if m.name == name { 0.6 } else { 0.1 }));
        }
        json!({"answers": {"lead_kwant": {"choice": name, "probabilities": probs}}})
    };
    let route = |prompt: &str, laya: &serde_json::Value| {
        let guess = kwnt::router::team::laya_guess(laya, &members);
        let p = kwnt::router::team::pick_member(prompt, "dev", &members, guess, "channel lead").unwrap();
        (p.kwant.name.clone(), p.basis)
    };
    // "plan" is only in Quinn's persona ("writes test plans"); Laya is sure it's Mia's
    assert_eq!(route("Plan our content calendar for next month", &laya_says("Mia")), ("Mia".into(), "skills"));
    // "review" is only in Dev's persona ("reviews code"); Laya is sure it's Dana's
    assert_eq!(route("Give the checkout flow a quick review", &laya_says("Dana")), ("Dana".into(), "skills"));
    // A near-tie from Laya doesn't override them
    let unsure = json!({"answers": {"lead_kwant": {"choice": "Mia", "probabilities": {"Mia": 0.3, "Quinn": 0.25, "Dev": 0.2, "Remy": 0.15, "Dana": 0.1}}}});
    assert_eq!(route("Plan our content calendar for next month", &unsure).0, "Quinn");
    // A word in a bot's title still beats Laya: that is the role the person named
    assert_eq!(route("Ask the designer about this", &laya_says("Mia")).0, "Dana");
}

// Follow-ups from Rigel's PR #28 review (laya-eval/rigel-check-28.md); h/hb ids are from eval-holdout.json.

#[test]
fn whole_word_keywords_still_match_possessives_hyphens_and_versions() {
    for (prompt, model) in [
        ("Explain Rust's borrow checker", "codex"),
        ("Why is Python's GIL a problem here?", "codex"),
        ("python3 is failing to import my module", "codex"),
        ("Explain this codebase to me", "codex"),
        ("I'm scripting a nightly backup", "codex"),
        ("Do a code-review of this diff", "codex"),
        ("Use Ollama's smallest model for this", "local_ollama"),
        ("Run it on-device, please", "local_ollama"),
    ] {
        assert_eq!(offline_model(prompt), model, "{prompt}");
    }
    use kwnt::router::fallback::heuristic_effort;
    assert_eq!(heuristic_effort("Give me a deep analysis of our churn", "chat"), "high");
    assert_eq!(heuristic_effort("Think deeply about this design", "chat"), "high");
    assert_eq!(heuristic_effort("Let's do a deep-dive on the auth flow", "chat"), "high");
    // The #28 traps stay fixed
    assert_ne!(offline_model("Open localhost:3000 and tell me why the proxy returns 502"), "local_ollama", "h01");
    assert_ne!(offline_model("Keep the app local-first and sync later"), "local_ollama");
    assert_eq!(offline_model("Write a punchy product description for our candle shop"), "claude", "m11");
    assert_eq!(heuristic_effort("Explain what a 'deep copy' is in Python in one line", "chat"), "low", "a03");
}

#[test]
fn a_private_browser_mode_is_not_a_request_for_the_local_model() {
    assert_ne!(
        offline_model("localStorage.setItem throws in Safari private mode. Can you review the code and fix the bug?"),
        "local_ollama",
        "h16"
    );
    assert_ne!(offline_model("Why is this Kotlin method private?"), "local_ollama");
    // The person's own private data still stays here
    assert_eq!(offline_model("Summarize my private medical lab results: total cholesterol 240, LDL 160, HDL 38"), "local_ollama", "m14");
    assert_eq!(offline_model("This is private data, keep it on this machine"), "local_ollama");
}

#[test]
fn a_negation_that_asks_not_to_skip_something_still_counts() {
    use kwnt::router::fallback::{blend_effort, heuristic_effort};
    assert_eq!(heuristic_effort("Don't ignore security warnings from npm audit", "chat"), "high");
    assert_eq!(blend_effort("Don't ignore security warnings from npm audit", "medium"), "high");
    assert_eq!(heuristic_effort("Please don't refactor anything, just tell me what time zone Paris is in", "chat"), "low", "a01");
    assert_ne!(heuristic_effort("Don't refactor the module yet, just rename it", "chat"), "high");
}

fn qa_team() -> Vec<Kwant> {
    let bot = |name: &str, title: &str, persona: &str| {
        let mut k = agent(&name.to_lowercase(), name, title);
        k.persona_prompt = persona.into();
        k
    };
    vec![
        bot("Sam", "Software Engineer", "You build and maintain backend and frontend features."),
        bot("Quincy", "QA Engineer", "You test software, find defects and write test plans."),
        bot("Pia", "Product Designer", "You design screens and user flows."),
    ]
}

#[test]
fn a_test_plan_goes_to_qa_even_when_laya_is_sure_it_is_the_engineers() {
    let members = qa_team();
    // Laya's real answer for hb17: Sam 0.58, Quincy 0.29
    let laya = json!({"answers": {"lead_kwant": {"choice": "Sam", "probabilities": {"Sam": 0.58, "Quincy": 0.29, "Pia": 0.13}}}});
    let route = |prompt: &str| {
        let guess = kwnt::router::team::laya_guess(&laya, &members);
        kwnt::router::team::pick_member(prompt, "sam", &members, guess, "channel lead").unwrap().kwant.name.clone()
    };
    assert_eq!(route("Write a test plan for the payment feature"), "Quincy", "hb17");
    assert_eq!(route("Write test cases for the password reset flow"), "Quincy");
    assert_eq!(route("Draft acceptance tests for the refund flow"), "Quincy");
    // Code work still goes to the engineer
    assert_eq!(route("Implement pagination in the users API"), "Sam", "hb16");
    // #28's protection holds: a word only in a persona doesn't beat a confident Laya
    let members = eval_team();
    let mia = json!({"answers": {"lead_kwant": {"choice": "Mia", "probabilities": {"Mia": 0.6, "Dev": 0.1, "Remy": 0.1, "Dana": 0.1, "Quinn": 0.1}}}});
    let guess = kwnt::router::team::laya_guess(&mia, &members);
    let pick = |p: &str| kwnt::router::team::pick_member(p, "dev", &members, guess, "channel lead").unwrap().kwant.name.clone();
    assert_eq!(pick("Plan our content calendar for next month"), "Mia");
    assert_eq!(pick("Write a test plan for our ad campaign A/B test"), "Mia", "x02");
}

#[test]
fn roles_come_from_titles_in_any_spelling_not_from_names() {
    let pick = |members: &[Kwant], prompt: &str| {
        let lead = members.last().unwrap().id.clone();
        kwnt::router::team::pick_member(prompt, &lead, members, None, "channel lead").unwrap().kwant.name.clone()
    };
    let mia = agent("mia", "Mia", "Marketing Manager");
    let sam = agent("sam", "Sam", "Software Engineer");
    assert_eq!(pick(&[agent("zed", "Zed", "Front End Developer"), mia.clone()], "Fix the button css layout"), "Zed");
    assert_eq!(pick(&[agent("zed", "Zed", "SDET"), sam.clone()], "Write regression tests for checkout"), "Zed");
    assert_eq!(pick(&[agent("zed", "Zed", "Q.A. Engineer"), sam.clone()], "Write regression tests for checkout"), "Zed");
    // A bot named Art is a software engineer by title, not a designer by name
    assert_eq!(pick(&[agent("art", "Art", "Software Engineer"), mia.clone()], "Fix the crash in the parser"), "Art");
    // A name still counts when the title names no role
    assert_eq!(pick(&[agent("ui", "Frontend", "Helper"), mia.clone()], "Fix the button css layout"), "Frontend");
    // "strategy" next to a marketing word is a marketing strategy, not a trading one (hb05)
    let members = eval_team();
    let p = kwnt::router::team::pick_member("Plan our SEO keyword strategy for Q4", "dev", &members, None, "channel lead").unwrap();
    assert_eq!(p.kwant.name, "Mia", "hb05");
}
