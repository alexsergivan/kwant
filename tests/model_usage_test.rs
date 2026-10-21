//! `GET /api/kwants/:id/model-usage`: per-bot model calls and tokens, as the bot's usage view shows them.

mod common;

use axum::http::StatusCode;
use common::*;
use kwnt::db::usage::{NewUsage, PURPOSE_REPLY, PURPOSE_SUMMARY};

fn call_row(kwant: &str, purpose: &'static str, fresh: i64) -> NewUsage {
    NewUsage {
        kwant_id: kwant.into(),
        context_id: "ctx".into(),
        purpose,
        model: "claude-opus-5-5".into(),
        fresh_input: fresh,
        cache_read: 4000,
        cache_write: 0,
        output: 12,
        reported: true,
        tools_chars: 900,
        system_chars: 5000,
        messages_chars: 700,
    }
}

#[tokio::test]
async fn a_bots_usage_lists_its_calls_and_totals() {
    let env = setup().await;
    for (purpose, fresh) in [(PURPOSE_REPLY, 30), (PURPOSE_REPLY, 40), (PURPOSE_SUMMARY, 300)] {
        env.state.db.record_usage(&call_row(&env.a, purpose, fresh)).await.unwrap();
    }
    env.state.db.record_usage(&call_row(&env.b, PURPOSE_REPLY, 1)).await.unwrap();

    let (s, body) = call(&env.app, "GET", &format!("/api/kwants/{}/model-usage?limit=2", env.a), None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["kwant_id"], env.a.as_str());
    assert_eq!(body["totals"]["calls"], 3);
    assert_eq!(body["totals"]["fresh_input_tokens"], 370);
    assert_eq!(body["totals"]["cache_read_tokens"], 12000);
    let reply = body["by_purpose"].as_array().unwrap().iter().find(|p| p["purpose"] == "reply").unwrap().clone();
    assert_eq!((reply["calls"].as_i64(), reply["fresh_input_tokens"].as_i64()), (Some(2), Some(70)));
    let recent = body["recent"].as_array().unwrap();
    assert_eq!(recent.len(), 2, "limit applies to the list, not the totals");
    assert_eq!(recent[0]["purpose"], "summary", "newest first");
    for key in ["context_id", "model", "cache_write_tokens", "output_tokens", "reported", "tools_chars", "system_chars", "messages_chars", "created_at"] {
        assert!(recent[0].get(key).is_some(), "row has {key}");
    }

    // The delete dialog's counts stay at the old path
    let (s, counts) = call(&env.app, "GET", &format!("/api/kwants/{}/usage", env.a), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(counts.get("totals").is_none(), "{counts}");

    let (s, _) = call(&env.app, "GET", "/api/kwants/no-such-bot/model-usage", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, body) = call(&env.app, "GET", &format!("/api/kwants/{}/model-usage?since=2999-01-01T00:00:00Z", env.a), None).await;
    assert_eq!((s, body["totals"]["calls"].as_i64()), (StatusCode::OK, Some(0)));
}
