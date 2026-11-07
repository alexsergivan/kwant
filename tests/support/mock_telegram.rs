//! A mock Telegram Bot API for the Telegram bridge tests (next-077 §G). Tests never reach
//! api.telegram.org: kwant's client is pointed here with `KWNT_TELEGRAM_API_BASE` (honoured in
//! debug and test builds only, decision Q-G2), and every token is fake ([`fake_token`]).
//!
//! ```ignore
//! #[path = "support/mock_telegram.rs"]
//! mod mock_telegram;
//! let tg = mock_telegram::shared();                  // one per test binary, on its own thread
//! let bot = tg.bot(7);                                // a fresh fake bot: token, id, username
//! ... PUT /api/connections/telegram/token {"token": bot.token} ...
//! tg.text(&bot, &PERSON, "/start <code>");            // the person writes to the bot
//! let sent = tg.wait_send(&bot, |c| c.text().contains("Paired with kwant")).await;
//! ```
//!
//! - One mock serves the whole test binary (the base URL is a process-wide variable), so each
//!   test uses its own bot number; everything is kept per token.
//! - `getUpdates` answers the updates at or after `offset` (earlier ones are confirmed and
//!   dropped, as Telegram does), waiting up to a second for a new one.
//! - A token the mock doesn't know answers 404 Not Found (Telegram's answer for a made-up
//!   token); a revoked one answers 401.
//! - [`MockTelegram::fault`] queues error answers for one method of one bot.
//! - Every call is kept with its JSON body ([`MockTelegram::calls`]).

#![allow(dead_code)]

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// The variable kwant reads (debug and test builds only).
pub const API_BASE_ENV: &str = "KWNT_TELEGRAM_API_BASE";

/// A Telegram account writing to the bot.
#[derive(Clone, Copy, Debug)]
pub struct Person {
    pub id: i64,
    pub first_name: &'static str,
    pub username: &'static str,
}

/// The person who pairs.
pub const PERSON: Person = Person { id: 4242, first_name: "Oleks", username: "oleks_test" };
/// Someone else who found the bot.
pub const STRANGER: Person = Person { id: 6666, first_name: "Mallory", username: "mallory_test" };

/// A fake bot: its token (fake, built at runtime), id and username.
#[derive(Clone, Debug)]
pub struct Bot {
    pub token: String,
    pub id: i64,
    pub username: String,
    pub name: String,
}

/// A BotFather-shaped token that no Telegram server knows. Built at runtime, so no secret
/// scanner sees a token in the source.
pub fn fake_token(n: u32) -> (i64, String) {
    let id = 7_000_000_000i64 + i64::from(n);
    (id, format!("{id}:TESTONLY-{}", format!("{n:0>6}").repeat(4)))
}

/// One call kwant made.
#[derive(Clone, Debug)]
pub struct Call {
    pub method: String,
    pub body: Value,
}

impl Call {
    pub fn text(&self) -> &str {
        self.body["text"].as_str().unwrap_or_default()
    }

    pub fn chat(&self) -> i64 {
        self.body["chat_id"].as_i64().unwrap_or_default()
    }

    /// Every button's callback data.
    pub fn callbacks(&self) -> Vec<String> {
        self.buttons().iter().filter_map(|b| b["callback_data"].as_str().map(str::to_string)).collect()
    }

    /// Every button's URL.
    pub fn urls(&self) -> Vec<String> {
        self.buttons().iter().filter_map(|b| b["url"].as_str().map(str::to_string)).collect()
    }

    pub fn buttons(&self) -> Vec<Value> {
        self.body["reply_markup"]["inline_keyboard"]
            .as_array()
            .map(|rows| rows.iter().filter_map(Value::as_array).flatten().cloned().collect())
            .unwrap_or_default()
    }

    /// The id of the message a `sendMessage` made (kept by the mock), or an edit's target.
    pub fn message_id(&self) -> i64 {
        self.body["message_id"].as_i64().or_else(|| self.body["__sent_id"].as_i64()).unwrap_or_default()
    }
}

#[derive(Default)]
struct BotState {
    id: i64,
    username: String,
    name: String,
    revoked: bool,
    next_update: i64,
    next_message: i64,
    updates: VecDeque<Value>,
    calls: Vec<Call>,
    faults: HashMap<String, VecDeque<(u16, Value)>>,
}

#[derive(Default)]
struct Inner {
    bots: Mutex<HashMap<String, BotState>>,
    /// Requests whose path wasn't `/bot<token>/<method>`.
    strays: Mutex<Vec<String>>,
    wake: tokio::sync::Notify,
}

pub struct MockTelegram {
    pub base: String,
    inner: Arc<Inner>,
}

static MOCK: OnceLock<MockTelegram> = OnceLock::new();

/// The binary's mock, started on first use on its own thread (each test has its own runtime),
/// with `KWNT_TELEGRAM_API_BASE` pointing at it.
pub fn shared() -> &'static MockTelegram {
    MOCK.get_or_init(|| {
        let inner = Arc::new(Inner::default());
        let served = inner.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("a runtime");
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a port");
                tx.send(listener.local_addr().expect("an address").port()).expect("the port");
                let app = Router::new().route("/:bot/:method", post(handle)).fallback(stray).with_state(served);
                let _ = axum::serve(listener, app).await;
            });
        });
        let port = rx.recv().expect("the mock started");
        let base = format!("http://127.0.0.1:{port}");
        std::env::set_var(API_BASE_ENV, &base);
        MockTelegram { base, inner }
    })
}

fn tg_error(status: u16, description: &str) -> (u16, Value) {
    (status, json!({"ok": false, "error_code": status, "description": description}))
}

fn reply((status, body): (u16, Value)) -> Response {
    (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(body)).into_response()
}

async fn stray(State(inner): State<Arc<Inner>>, uri: axum::http::Uri) -> Response {
    inner.strays.lock().unwrap().push(uri.path().to_string());
    reply(tg_error(404, "Not Found"))
}

async fn handle(State(inner): State<Arc<Inner>>, Path((bot, method)): Path<(String, String)>, body: Bytes) -> Response {
    let Some(token) = bot.strip_prefix("bot").map(str::to_string) else {
        inner.strays.lock().unwrap().push(format!("/{bot}/{method}"));
        return reply(tg_error(404, "Not Found"));
    };
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    // getUpdates may wait for an update; everything else answers at once
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        let notified = inner.wake.notified();
        {
            let mut bots = inner.bots.lock().unwrap();
            let Some(b) = bots.get_mut(&token) else {
                return reply(tg_error(404, "Not Found"));
            };
            b.calls.push(Call { method: method.clone(), body: body.clone() });
            if b.revoked {
                return reply(tg_error(401, "Unauthorized"));
            }
            if let Some(f) = b.faults.get_mut(&method).and_then(VecDeque::pop_front) {
                return reply(f);
            }
            match method.as_str() {
                "getMe" => {
                    return reply((200, json!({"ok": true, "result": {"id": b.id, "is_bot": true, "first_name": b.name, "username": b.username}})));
                }
                "getUpdates" => {
                    let offset = body["offset"].as_i64().unwrap_or(0);
                    while b.updates.front().is_some_and(|u| u["update_id"].as_i64().unwrap_or(0) < offset) {
                        b.updates.pop_front();
                    }
                    if !b.updates.is_empty() || tokio::time::Instant::now() >= deadline {
                        let list: Vec<Value> = b.updates.iter().cloned().collect();
                        // The call is kept once, not once per wait
                        return reply((200, json!({"ok": true, "result": list})));
                    }
                    b.calls.pop();
                }
                "sendMessage" => {
                    if body["text"].as_str().is_some_and(|t| t.chars().count() > 4096) {
                        return reply(tg_error(400, "Bad Request: message is too long"));
                    }
                    b.next_message += 1;
                    let id = b.next_message;
                    if let Some(c) = b.calls.last_mut() {
                        c.body["__sent_id"] = json!(id);
                    }
                    return reply((200, json!({"ok": true, "result": {"message_id": id, "chat": {"id": body["chat_id"]}, "text": body["text"]}})));
                }
                "editMessageText" => return reply((200, json!({"ok": true, "result": {"message_id": body["message_id"]}}))),
                "answerCallbackQuery" => return reply((200, json!({"ok": true, "result": true}))),
                _ => return reply(tg_error(404, "Not Found")),
            }
        }
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let _ = tokio::time::timeout(left, notified).await;
    }
}

impl MockTelegram {
    /// A fresh fake bot `n` (each test its own number).
    pub fn bot(&self, n: u32) -> Bot {
        let (id, token) = fake_token(n);
        let username = format!("kwant_test{n}_bot");
        let name = format!("kwant test {n}");
        self.inner.bots.lock().unwrap().insert(
            token.clone(),
            BotState { id, username: username.clone(), name: name.clone(), next_update: 100, next_message: 1000, ..BotState::default() },
        );
        Bot { token, id, username, name }
    }

    /// BotFather revoked the token: every call answers 401 from now on.
    pub fn revoke(&self, bot: &Bot) {
        self.inner.bots.lock().unwrap().get_mut(&bot.token).expect("a mock bot").revoked = true;
    }

    /// The next `count` calls of `method` answer `status` with Telegram's error shape (429
    /// carries `retry_after: 1`).
    pub fn fault(&self, bot: &Bot, method: &str, status: u16, count: usize) {
        let mut bots = self.inner.bots.lock().unwrap();
        let b = bots.get_mut(&bot.token).expect("a mock bot");
        for _ in 0..count {
            let mut f = tg_error(status, if status == 409 { "Conflict: terminated by other getUpdates request" } else { "Error" });
            if status == 429 {
                f.1["parameters"] = json!({"retry_after": 1});
            }
            b.faults.entry(method.to_string()).or_default().push_back(f);
        }
    }

    /// Queues an update (its `update_id` is added) and returns the id.
    pub fn inject(&self, bot: &Bot, mut update: Value) -> i64 {
        let id = {
            let mut bots = self.inner.bots.lock().unwrap();
            let b = bots.get_mut(&bot.token).expect("a mock bot");
            b.next_update += 1;
            let id = b.next_update;
            update["update_id"] = json!(id);
            b.updates.push_back(update);
            id
        };
        self.inner.wake.notify_waiters();
        id
    }

    fn next_incoming(&self, bot: &Bot) -> i64 {
        let mut bots = self.inner.bots.lock().unwrap();
        let b = bots.get_mut(&bot.token).expect("a mock bot");
        b.next_message += 1;
        b.next_message
    }

    /// A private-chat message object from `who` (the chat id is the person's id, as in Telegram).
    pub fn message(&self, bot: &Bot, who: &Person, text: &str) -> Value {
        json!({
            "message_id": self.next_incoming(bot),
            "from": {"id": who.id, "is_bot": false, "first_name": who.first_name, "username": who.username},
            "chat": {"id": who.id, "type": "private", "first_name": who.first_name},
            "date": 1_700_000_000,
            "text": text,
        })
    }

    /// `who` writes `text` in their private chat with the bot; returns the update's id
    /// (for `wait_handled`).
    pub fn text(&self, bot: &Bot, who: &Person, text: &str) -> i64 {
        let m = self.message(bot, who, text);
        self.inject(bot, json!({"message": m}))
    }

    /// `who` replies to kwant's message `to` with `text`; returns the update's id.
    pub fn reply_to(&self, bot: &Bot, who: &Person, to: i64, text: &str) -> i64 {
        let mut m = self.message(bot, who, text);
        m["reply_to_message"] = json!({"message_id": to, "chat": {"id": who.id, "type": "private"}, "date": 1_700_000_000});
        self.inject(bot, json!({"message": m}))
    }

    /// `who` taps a button with `data` on kwant's message `message_id` (in `who`'s chat).
    pub fn tap(&self, bot: &Bot, who: &Person, message_id: i64, data: &str) -> String {
        let id = format!("cb{}", self.next_incoming(bot));
        self.inject(
            bot,
            json!({"callback_query": {
                "id": id,
                "from": {"id": who.id, "is_bot": false, "first_name": who.first_name},
                "message": {"message_id": message_id, "chat": {"id": who.id, "type": "private"}, "date": 1_700_000_000},
                "chat_instance": "1",
                "data": data,
            }}),
        );
        id
    }

    /// Every call kwant made with this bot's token, in order.
    pub fn calls(&self, bot: &Bot) -> Vec<Call> {
        self.inner.bots.lock().unwrap().get(&bot.token).map(|b| b.calls.clone()).unwrap_or_default()
    }

    pub fn calls_of(&self, bot: &Bot, method: &str) -> Vec<Call> {
        self.calls(bot).into_iter().filter(|c| c.method == method).collect()
    }

    pub fn sent(&self, bot: &Bot) -> Vec<Call> {
        self.calls_of(bot, "sendMessage")
    }

    /// Requests that weren't `/bot<token>/<method>`.
    pub fn strays(&self) -> Vec<String> {
        self.inner.strays.lock().unwrap().clone()
    }

    /// Waits (up to 30 s) for a call of `method` matching `pick`.
    pub async fn wait_call(&self, bot: &Bot, method: &str, pick: impl Fn(&Call) -> bool) -> Call {
        for _ in 0..300 {
            if let Some(c) = self.calls_of(bot, method).into_iter().find(|c| pick(c)) {
                return c;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("no {method} matched; calls: {:#?}", self.calls(bot).iter().filter(|c| c.method != "getUpdates").collect::<Vec<_>>());
    }

    pub async fn wait_send(&self, bot: &Bot, pick: impl Fn(&Call) -> bool) -> Call {
        self.wait_call(bot, "sendMessage", pick).await
    }

    pub async fn wait_edit(&self, bot: &Bot, pick: impl Fn(&Call) -> bool) -> Call {
        self.wait_call(bot, "editMessageText", pick).await
    }

    /// Waits until kwant has fetched updates past `update_id` (it has handled it).
    pub async fn wait_handled(&self, bot: &Bot, update_id: i64) {
        for _ in 0..300 {
            if self.calls_of(bot, "getUpdates").iter().any(|c| c.body["offset"].as_i64().unwrap_or(0) > update_id) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("kwant never got past update {update_id}");
    }
}
