//! Connections: `GET /api/connections` lists MCP servers, engine accounts, saved website logins
//! and Loose ends sources with status, last check and the bots that use them; Composio is an
//! optional connector, off when added, and never shows its key. Person-only.

mod common;

use axum::http::StatusCode;
use chrono::Utc;
use common::*;
use kwnt::db::{spaces::SpaceId, zombie::SourceRow};
use kwnt::sandbox::credentials::CredentialStore;
use kwnt::server::{continuity::SelfScope, mcp_runtime};
use serde_json::{json, Value};

fn find<'a>(list: &'a Value, id: &str) -> &'a Value {
    list["connections"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap_or_else(|| panic!("{id} not in {list}"))
}

fn user_ids(c: &Value) -> Vec<String> {
    c["used_by"].as_array().unwrap().iter().map(|u| u["id"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn connections_list_everything_with_status_and_users() {
    let env = setup().await;

    // An MCP server whose test fails: status error with a last check.
    let (s, server) = call(
        &env.app,
        "POST",
        "/api/mcp/servers",
        Some(json!({"name": "broken", "transport": "stdio", "command": "false", "args": ["--token", "sk-hidden-123"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{server}");
    let mcp_id = server["id"].as_str().unwrap().to_string();
    let (_, before) = call(&env.app, "GET", "/api/connections", None).await;
    let c = find(&before, &format!("mcp:{mcp_id}"));
    assert_eq!(c["status"], "unknown");
    assert!(c["last_check"].is_null());
    assert!(user_ids(c).contains(&env.a) && user_ids(c).contains(&env.b), "every bot in the space: {c}");
    let (_, tested) = call(&env.app, "POST", &format!("/api/mcp/servers/{mcp_id}/test"), None).await;
    assert_eq!(tested["ok"], false, "{tested}");

    // A saved login for bot a, and a mail source of Loose ends.
    env.state.credentials.save(&env.a, "https://github.com", "octo", "pw-never-listed").await.unwrap();
    env.state
        .db
        .zombie_put_source(&SourceRow {
            id: "src1".into(),
            kind: "imap".into(),
            label: "Work mail".into(),
            config: json!({"host": "imap.example.com", "port": 993, "username": "me@example.com", "folders": ["INBOX"]}).to_string(),
            enabled: true,
            last_scan_at: None,
            last_status: None,
            last_error: None,
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let (s, list) = call(&env.app, "GET", "/api/connections", None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let text = list.to_string();
    assert!(!text.contains("pw-never-listed") && !text.contains("sk-hidden-123"), "no secret in the list: {text}");

    let c = find(&list, &format!("mcp:{mcp_id}"));
    assert_eq!(c["kind"], "mcp");
    assert_eq!(c["status"], "error", "{c}");
    assert!(c["last_check"].is_string(), "the test is the last check: {c}");
    assert_eq!(c["detail"], "false", "the command only, never its arguments");
    assert_eq!(c["logins_stored"], "kwnt");

    let login = list["connections"].as_array().unwrap().iter().find(|c| c["kind"] == "browser_login").expect("the saved login");
    assert_eq!(login["name"], "github.com");
    assert_eq!(login["logins_stored"], "vault");
    assert_eq!(user_ids(login), vec![env.a.clone()], "only the bot it was saved for");
    assert_eq!(list["vault"]["status"], "ok");
    assert_eq!(list["vault"]["logins"], 1);

    let mail = find(&list, "source:src1");
    assert_eq!(mail["kind"], "email");
    assert_eq!(mail["status"], "unknown");
    assert_eq!(mail["detail"], "me@example.com @ imap.example.com");

    assert!(list["connections"].as_array().unwrap().iter().any(|c| c["kind"] == "account"), "engine accounts are listed");
    assert!(list["counts"]["error"].as_u64().unwrap() >= 1);

    // Composio is in the catalog, optional and not added.
    let composio = &list["catalog"][0];
    assert_eq!(composio["id"], "composio");
    assert_eq!(composio["optional"], true);
    assert_eq!(composio["added"], false);
    assert_eq!(composio["logins_stored"], "composio");
    assert!(composio["warning"].as_str().unwrap().contains("Composio's servers"));
}

#[tokio::test]
async fn composio_is_added_off_and_keeps_its_key_hidden() {
    let env = setup().await;
    for bad in ["http://backend.composio.dev/v3/mcp/x", "https://composio.dev.evil.example/mcp", "https://example.com/mcp"] {
        let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"url": bad}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
    let (s, added) = call(
        &env.app,
        "POST",
        "/api/connections/composio",
        Some(json!({"url": "https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u1", "api_key": "ck-secret-999"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{added}");
    assert_eq!(added["server"]["enabled"], false, "off until the person switches it on");
    assert!(!added.to_string().contains("ck-secret-999"), "{added}");
    let id = added["connection_id"].as_str().unwrap().to_string();

    let (s, again) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"url": "https://backend.composio.dev/v3/mcp/abc/mcp"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{again}");

    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    assert!(!list.to_string().contains("ck-secret-999"));
    let c = find(&list, &id);
    assert_eq!(c["status"], "off");
    assert_eq!(c["logins_stored"], "composio");
    assert!(c["used_by"].as_array().unwrap().is_empty(), "no bot uses it while it is off");
    assert_eq!(list["catalog"][0]["added"], true);
    assert_eq!(list["catalog"][0]["connection_id"], id);
    assert_eq!(list["catalog"][0]["enabled"], false);

    // the stored server really is off: no bot gets its tools
    let server = env.state.db.list_mcp_servers(&SpaceId::default_space()).await.unwrap();
    assert!(server.iter().any(|s| s.name == "composio" && !s.enabled));
}

#[tokio::test]
async fn connections_are_person_only() {
    let env = setup().await;
    assert_eq!(call_remote(&env.app, "GET", "/api/connections").await, StatusCode::UNAUTHORIZED);
    assert_eq!(call_remote(&env.app, "POST", "/api/connections/composio").await, StatusCode::UNAUTHORIZED);

    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let folder = tempfile::tempdir().unwrap();
    let (_, token) =
        mcp_runtime::prepare_turn_with_self(&env.state, &bot, None, Some(folder.path().to_path_buf()), Some(scope)).await.unwrap();
    let bearer = format!("Bearer {token}");
    let (s, _) = call_with(&env.app, "GET", "/api/connections", None, &[("authorization", &bearer)]).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call_with(
        &env.app,
        "POST",
        "/api/connections/composio",
        Some(json!({"url": "https://backend.composio.dev/v3/mcp/abc/mcp"})),
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
