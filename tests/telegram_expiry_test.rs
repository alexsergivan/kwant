//! Telegram timers (next-077 §G3, A19) with the test clock moved forward
//! (`KWNT_TEST_NOW_OFFSET_SECS`, src/clock.rs). The clock is process-wide, so this binary runs
//! one test.

mod common;
#[path = "support/mock_telegram.rs"]
mod mock_telegram;

use axum::http::StatusCode;
use common::{call, setup, wait_for, Env};
use kwnt::drivers::events::RuntimeEvent;
use mock_telegram::PERSON;
use serde_json::{json, Value};
use std::time::Duration;

const BASE: &str = "/api/connections/telegram";

async fn status(env: &Env) -> Value {
    call(&env.app, "GET", BASE, None).await.1
}

async fn new_code(env: &Env) -> String {
    let (s, v) = call(&env.app, "POST", &format!("{BASE}/pair"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["qr_svg"].as_str().unwrap().starts_with("<svg"));
    v["link"].as_str().unwrap().split("start=").nth(1).unwrap().to_string()
}

#[tokio::test]
async fn the_code_expires_after_ten_minutes_and_the_reply_window_after_six_hours() {
    std::env::remove_var(kwnt::clock::OFFSET_ENV);
    let env = setup().await;
    let tg = mock_telegram::shared();
    let bot = tg.bot(100);
    let (s, v) = call(&env.app, "PUT", &format!("{BASE}/token"), Some(json!({"token": bot.token}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut events = env.state.event_sender.subscribe();

    // Ten minutes and a second later the code is gone: telegram.expired, back to token_saved
    let old = new_code(&env).await;
    assert_eq!(status(&env).await["state"], "pairing");
    std::env::set_var(kwnt::clock::OFFSET_ENV, "601");
    wait_for("expired", || async { status(&env).await["state"] == "token_saved" }).await;
    let mut expired = false;
    while let Ok(e) = events.try_recv() {
        expired |= matches!(e.unscoped(), RuntimeEvent::TelegramExpired { .. });
    }
    assert!(expired, "telegram.expired was sent");
    let last = tg.text(&bot, &PERSON, &format!("/start {old}"));
    tg.wait_handled(&bot, last).await;
    assert_eq!(status(&env).await["state"], "token_saved", "an expired code never pairs");

    // A fresh code (made on the moved clock) pairs
    let code = new_code(&env).await;
    tg.text(&bot, &PERSON, &format!("/start {code}"));
    tg.wait_send(&bot, |c| c.text().contains("Paired with kwant")).await;
    wait_for("paired", || async { status(&env).await["state"] == "paired" }).await;

    // Writing from Telegram opens the reply window; six hours on it closes by itself
    tg.text(&bot, &PERSON, "hello from Telegram");
    wait_for("window open", || async {
        env.state.db.telegram_link("spc_default").await.unwrap().unwrap().reply_window_from.is_some()
    })
    .await;
    std::env::set_var(kwnt::clock::OFFSET_ENV, (601 + 6 * 3600 + 5).to_string());
    wait_for("window closed", || async {
        env.state.db.telegram_link("spc_default").await.unwrap().unwrap().reply_window_from.is_none()
    })
    .await;
    std::env::remove_var(kwnt::clock::OFFSET_ENV);
    tokio::time::sleep(Duration::from_millis(10)).await;
}
