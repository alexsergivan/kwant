//! "Run Claude inside the VM": the switch and chips in `GET/PUT /api/settings/computer`
//! (design/ui.md's contract, Ada's D1).
mod common;

use axum::http::StatusCode;
use kwnt::sandbox::agent_vm::{broker::BrokerConfig, AgentVm, AgentsInVmConfig};
use serde_json::json;

#[tokio::test]
async fn the_switch_saves_and_the_chips_say_where_each_engine_runs() {
    let data = tempfile::tempdir().unwrap();
    let env = common::setup_with(|state| {
        // An image name that's never built: turning the switch on can't start a real download
        state.agent_vm = AgentVm::with_parts(
            data.path(),
            BrokerConfig::new(data.path().join("broker.log")),
            Some("kwnt-agent-settings-test:never-built".into()),
            AgentsInVmConfig::default(),
            None,
        );
    })
    .await;
    let (s, v) = common::call(&env.app, "GET", "/api/settings/computer", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["isolate_claude"], false, "off by default: {v}");
    assert_eq!(v["agents"]["claude"], json!({"state": "off", "reason": null, "detail": null, "progress": null}));
    for other in ["codex", "grok"] {
        assert_eq!(v["agents"][other]["state"], "not_isolated");
        assert_eq!(v["agents"][other]["reason"], "not_supported_yet");
    }
    let available = v["isolate_claude_toggle"]["available"].as_bool().unwrap();
    if cfg!(target_os = "macos") {
        assert_eq!(v["isolate_claude_toggle"]["reason"], "mac_later");
    }
    // Isolation is about Claude alone: `isolates` never names the CLI agents
    assert!(!v["isolates"].as_array().unwrap().iter().any(|i| i == "cli_agents"), "{v}");

    // Nothing to change, and a wrong type, are refused
    assert_eq!(common::call(&env.app, "PUT", "/api/settings/computer", Some(json!({}))).await.0, StatusCode::BAD_REQUEST);
    let (s, _) = common::call(&env.app, "PUT", "/api/settings/computer", Some(json!({"isolate_claude": "yes"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Only the person can turn it on
    assert_eq!(common::call_remote(&env.app, "PUT", "/api/settings/computer").await, StatusCode::UNAUTHORIZED);

    let (s, v) = common::call(&env.app, "PUT", "/api/settings/computer", Some(json!({"isolate_claude": true}))).await;
    if available {
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["isolate_claude"], true);
        let claude = &v["agents"]["claude"];
        if v["mode"] == "local" {
            // D1: in local mode the switch is kept but does nothing, and the chip says so
            assert_eq!((claude["state"].as_str(), claude["reason"].as_str()), (Some("not_isolated"), Some("local_mode")), "{v}");
        } else {
            // The test image is never built: it's getting ready or it failed, never "isolated"
            assert!(["preparing", "unavailable"].contains(&claude["state"].as_str().unwrap()), "{v}");
        }
    } else {
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let reason = v["error"].as_str().unwrap_or_default();
        assert!(["mac_later", "docker_unavailable", "not_supported_yet"].contains(&reason), "{v}");
        let (_, v) = common::call(&env.app, "GET", "/api/settings/computer", None).await;
        assert_eq!(v["isolate_claude"], false);
    }

    // Turning it off always works
    let (s, v) = common::call(&env.app, "PUT", "/api/settings/computer", Some(json!({"isolate_claude": false}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["isolate_claude"], false);
    assert_eq!(v["agents"]["claude"]["state"], "off");
}
