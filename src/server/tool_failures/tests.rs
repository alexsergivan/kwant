use super::*;

fn servers() -> Vec<Server> {
    vec![
        Server { id: "s-gh".into(), name: "github".into(), composio: false },
        Server { id: "s-co".into(), name: "composio".into(), composio: true },
    ]
}

fn gh() -> Failure {
    classify("mcp__github__list_issues", &json!({}), "HTTP 401 Unauthorized: bad credentials", &servers()).unwrap()
}

fn down(key: &str) -> Failure {
    Failure {
        conn: Connection { key: key.into(), label: key.into(), server_id: Some("s".into()) },
        cause: Cause::Down,
        tool: None,
        detail: None,
    }
}

/// A 401 is a sign-in failure of that server, and its line links to Connections.
#[test]
fn a_401_is_a_sign_in_with_a_fix_link() {
    let f = gh();
    assert_eq!(f.cause, Cause::SignIn);
    assert_eq!(f.conn, Connection { key: "mcp:s-gh".into(), label: "github".into(), server_id: Some("s-gh".into()) });
    assert_eq!(
        text(&f, "Ben"),
        "github stopped working for Ben: its sign-in expired or was refused. Reconnect it in Settings › Connections."
    );
    let m = meta(&f, "k1", "Ben", "t1");
    assert_eq!(m["kind"], KIND);
    assert_eq!(m["needs_you"], true);
    assert_eq!(m["fix"], json!({"label": "Reconnect", "opens": "connections", "connection_id": "mcp:s-gh"}));
    assert_eq!(m["cause"], "sign_in");
    assert!(m["resolved_at"].is_null());
    // 403 and an expired token too; a number that merely contains 401 doesn't count
    for why in ["403 Forbidden", "invalid_grant: Token has expired", "error: Unauthorized"] {
        assert_eq!(classify("mcp__github__x", &json!({}), why, &servers()).unwrap().cause, Cause::SignIn, "{why}");
    }
    assert!(classify("mcp__github__x", &json!({}), "issue 14012 not found", &servers()).is_none());
}

/// A tool's own error (bad arguments, not found) isn't the connection's: no line.
#[test]
fn a_tools_own_error_is_not_a_connection_failure() {
    for why in ["", "invalid arguments: repo is required", "Not Found", "rate limit: try in 2 minutes"] {
        assert!(classify("mcp__github__get_repo", &json!({}), why, &servers()).is_none(), "{why}");
    }
    // Engines' own tools and unknown servers aren't connections
    assert!(classify("Bash", &json!({}), "401", &servers()).is_none());
    assert!(classify("mcp__unknown__x", &json!({}), "401", &servers()).is_none());
}

/// A Composio app that lost its account needs connecting again, keyed by the app, not the server.
#[test]
fn composio_failures_are_per_app_and_need_a_reconnect() {
    let f = classify("mcp__composio__GMAIL_SEND_EMAIL", &json!({}), "Connected account not found for toolkit gmail", &servers()).unwrap();
    assert_eq!(f.cause, Cause::Reconnect);
    assert_eq!(f.conn.key, "composio:gmail");
    assert_eq!(f.conn.label, "Gmail");
    assert_eq!(line(&f, "Ben"), "Gmail stopped working for Ben: it needs to be connected again.");
    let args = json!({"tools": [{"tool_slug": "SLACK_SEND_MESSAGE", "arguments": {}}]});
    let f = classify("mcp__composio__COMPOSIO_MULTI_EXECUTE_TOOL", &args, "401 Unauthorized", &servers()).unwrap();
    assert_eq!((f.conn.key.as_str(), f.cause), ("composio:slack", Cause::Reconnect));
    assert_eq!(meta(&f, "k", "Ben", "t")["fix"]["connection_id"], "mcp:s-co");
    // A meta tool with no app falls back to the server
    let f = classify("mcp__composio__COMPOSIO_SEARCH_TOOLS", &json!({}), "503 Service Unavailable", &servers()).unwrap();
    assert_eq!((f.conn.key.as_str(), f.cause), ("mcp:s-co", Cause::Down));
}

/// kwant's own tools fetch the web for the bot: a page's 401 isn't kwant failing.
#[test]
fn kwants_own_tools_count_only_their_transport() {
    assert!(classify("mcp__kwnt_tools__web_fetch", &json!({}), "HTTP 401 from example.com", &servers()).is_none());
    let f = classify("mcp__kwnt_tools__web_fetch", &json!({}), "tool call failed: transport closed", &servers()).unwrap();
    assert_eq!((f.conn.key.as_str(), f.cause, f.conn.server_id.clone()), ("kwnt", Cause::Down, None));
    assert_eq!(text(&f, "Ben"), "kwant's own tools stopped responding for Ben.");
    assert!(meta(&f, "k", "Ben", "t")["fix"].is_null());
}

/// A server that didn't start: auth text makes it a sign-in, anything else "didn't start".
#[test]
fn a_server_that_did_not_start() {
    let f = not_started("github: it failed to start", &servers()).unwrap();
    assert_eq!(f.cause, Cause::NotStarted);
    assert_eq!(text(&f, "Ben"), "github didn't start for Ben, so its tools weren't available. Check it in Settings › Connections.");
    assert_eq!(not_started("github: authentication required", &servers()).unwrap().cause, Cause::SignIn);
    assert!(not_started("nobody: failed", &servers()).is_none());
}

/// Two failures of one connection give one line; a failure a retry fixed gives none.
#[test]
fn one_line_per_connection_per_turn() {
    let mut w = Watch::default();
    let Next::Wait(gen) = w.failed(gh()) else { panic!("first failure waits") };
    assert_eq!(w.failed(gh()), Next::Post(gh()));
    assert_eq!(w.failed(gh()), Next::Nothing);
    assert_eq!(w.due("mcp:s-gh", gen), None);
    assert!(w.finish().is_empty());

    // Failure, then a retry that works: nothing
    let mut w = Watch::default();
    let Next::Wait(gen) = w.failed(gh()) else { panic!() };
    assert!(w.succeeded("mcp:s-gh"));
    assert_eq!(w.due("mcp:s-gh", gen), None);
    assert!(w.finish().is_empty());
    assert!(!w.was_posted("mcp:s-gh"));

    // A failure, a retry fixing it, then failing again later: the new failure gets its own grace
    let Next::Wait(gen2) = w.failed(gh()) else { panic!() };
    assert_eq!(w.due("mcp:s-gh", gen), None, "an old timer doesn't post the new failure");
    assert_eq!(w.due("mcp:s-gh", gen2), Some(gh()));
    assert!(w.was_posted("mcp:s-gh"));
}

/// The turn ending posts what still waits, oldest first; a server that didn't start posts at once.
#[test]
fn the_end_of_the_turn_posts_what_waits() {
    let mut w = Watch::default();
    assert!(matches!(w.failed(down("b")), Next::Wait(_)));
    assert!(matches!(w.failed(down("a")), Next::Wait(_)));
    let ns = Failure { cause: Cause::NotStarted, ..down("c") };
    assert_eq!(w.failed(ns.clone()), Next::Post(ns));
    let keys: Vec<String> = w.finish().into_iter().map(|f| f.conn.key).collect();
    assert_eq!(keys, ["b", "a"]);
    assert_eq!(w.failed(down("a")), Next::Nothing);
}

/// A runaway turn can't track failures without bound.
#[test]
fn failures_per_turn_are_capped() {
    let mut w = Watch::default();
    for i in 0..MAX_KEYS {
        assert!(matches!(w.failed(down(&format!("k{i}"))), Next::Wait(_)));
    }
    assert_eq!(w.failed(down("one-more")), Next::Nothing);
}

fn kwant(id: &str, name: &str) -> crate::db::models::Kwant {
    serde_json::from_value(json!({
        "id": id, "name": name, "title": "", "persona_prompt": "", "avatar_sprite": "", "driver_type": "claude",
        "preferred_model": "", "temperature": 0.5, "max_tokens": 1000, "level": 1, "turns_completed": 0, "created_at": null,
    }))
    .unwrap()
}

fn line_msg(id: &str, kwant_id: &str, key: &str, at: DateTime<Utc>, needs_you: bool, resolved: bool) -> Message {
    let mut v = meta(&gh(), kwant_id, "Ben", "t");
    v["key"] = json!(key);
    v["needs_you"] = json!(needs_you);
    if resolved {
        v["resolved_at"] = json!(at);
    }
    serde_json::from_value(json!({
        "id": id, "channel_id": null, "direct_kwant_id": kwant_id, "sender_role": "kwant", "sender_id": "Ben",
        "text": "x", "reasoning_content": null, "tool_calls": null, "tokens_used": 0, "created_at": at,
        "meta": v.to_string(),
    }))
    .unwrap()
}

/// "Needs you" lists open sign-in lines: newest per bot and connection, not resolved, not old,
/// not "down" lines, not bots outside the space.
#[test]
fn needs_you_lists_open_sign_in_lines() {
    let now = Utc::now();
    let kwants = [kwant("k1", "Ben"), kwant("k2", "Ann")];
    let lines = [
        line_msg("m1", "k1", "mcp:s-gh", now - Span::minutes(1), true, false),
        line_msg("m2", "k1", "mcp:s-gh", now - Span::minutes(5), true, false),
        line_msg("m3", "k2", "mcp:s-gh", now - Span::minutes(2), true, true),
        line_msg("m4", "k2", "composio:gmail", now - Span::days(NEEDS_YOU_DAYS + 1), true, false),
        line_msg("m5", "k2", "mcp:s-x", now - Span::minutes(3), false, false),
        line_msg("m6", "k9", "mcp:s-gh", now - Span::minutes(3), true, false),
    ];
    let items = needs_you_items(&lines, &kwants, now);
    assert_eq!(items.len(), 1, "{items:?}");
    let item = &items[0].1;
    assert_eq!(item["kind"], "connection");
    assert_eq!(item["message_id"], "m1");
    assert_eq!(item["kwant_name"], "Ben");
    assert_eq!(item["direct_kwant_id"], "k1");
    assert_eq!(item["fix"]["opens"], "connections");
    assert_eq!(item["text"], "github stopped working for Ben: its sign-in expired or was refused.");
}

/// A long reason is cut before it is kept.
#[test]
fn reasons_are_cut() {
    let long = format!("401 Unauthorized {}", "x".repeat(2000));
    let f = classify("mcp__github__x", &json!({}), &long, &servers()).unwrap();
    assert!(f.detail.unwrap().chars().count() <= MAX_DETAIL + 1);
}
