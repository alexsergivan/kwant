//! pa13a: the front desk kit (Guest Guide). Each guest has one private conversation with the
//! host's bot: a guest's `say` is saved here and queued, and the server's own runtime
//! ([`crate::server::desk_turns`]) answers it as an outsider turn, one turn at a time per guest,
//! through [`Kit::server_event`]. Nothing here calls a model; this file only keeps the
//! conversations, the limits and the views.
//!
//! Privacy: a guest's view holds only their own conversation; `/info` and the lobby hold only
//! the host's public facts. No guest word goes into a kit event (the core saves those) or a log.
//!
//! Limits: 20 messages an hour per guest (the 21st is a 429 `slow_down`), 1,000 characters a
//! message (413 `too_long`), `max_guests` guests (409 `guest_max`), and `daily_cap` bot replies
//! a day for the whole pocket: past it a message is saved and waits for the host (no model call).
//! The host's `set_cap` changes that limit on a running desk (it wins over the spec's).
//!
//! Retention: a guest's data goes 7 days after their last message (their row, keys, events,
//! bot thread and conversation, swept once a minute); the pocket goes 7 days after it ends.

use super::kits::{Kit, KitCtx, KitError, KitRules, Player, Retention, Viewer, DEFAULT_MAX_PLAYERS};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const ID: &str = "desk";

/// Messages a guest may send in any hour.
pub const PER_HOUR: usize = 20;
/// Characters a guest message may have.
pub const MAX_CHARS: usize = 1000;
/// Days a guest's data stays after their last message; also days after the end for the pocket.
pub const KEEP_DAYS: i64 = 7;
/// Characters of the house guide.
pub const GUIDE_MAX: usize = 12_000;
/// Bot replies a day for the whole pocket: the default and the bounds.
pub const DAILY_CAP: u64 = 50;
pub const DAILY_CAP_MIN: u64 = 10;
pub const DAILY_CAP_MAX: u64 = 200;
pub const MAX_GUESTS: u64 = 30;
pub const MAX_STARTERS: usize = 3;
pub const STARTER_MAX: usize = 24;
/// A hand-off reason as the card shows it.
pub const REASON_MAX: usize = 120;
/// What one bot or host reply may hold.
pub const REPLY_MAX: usize = 4000;
/// Messages kept per guest (the oldest go first; the bot's thread keeps the whole conversation).
pub const KEEP_MESSAGES: usize = 80;
/// Characters of a guest's last message on the card.
pub const LAST_MAX: usize = 90;
/// The bot's avatar style.
pub const SPRITE: &str = "desk";

pub const GUEST: &str = "guest";
pub const BOT: &str = "bot";
pub const HOST: &str = "host";
pub const NOTE: &str = "note";

pub const HANDOFF: &str = "handoff";
pub const CAPPED: &str = "daily_cap";
/// The note when no reply came at all (the bot is gone, had nothing it could send, or its turn
/// failed): the guest page says the bot can't answer and the host has the message. The bot's own
/// `[[person]]` gets [`HANDOFF`].
pub const HOST_AWAY: &str = "host_away";
/// The host's label for a guest waiting past the daily cap (Mira's card chip, state 8).
pub const CAP_WHY: &str = "Daily limit reached";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct St {
    v: u32,
    /// The box's local date the counts below are for.
    #[serde(default)]
    day: String,
    #[serde(default)]
    replies_today: u64,
    #[serde(default)]
    messages_today: u64,
    #[serde(default)]
    next: u64,
    #[serde(default)]
    guests: BTreeMap<String, G>,
    /// The host's `set_cap` (10 to 200); it wins over the spec's `daily_cap`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cap: Option<u64>,
    /// What the bot's turns cost, summed over the turns whose engine said (`priced_turns`).
    #[serde(default)]
    spent_usd: f64,
    #[serde(default)]
    priced_turns: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct G {
    #[serde(default)]
    msgs: Vec<Msg>,
    /// When this guest's messages of the last hour came.
    #[serde(default)]
    sent: Vec<DateTime<Utc>>,
    /// The guest's last message (retention counts from here; a host reply doesn't move it).
    #[serde(default)]
    last_at: Option<DateTime<Utc>>,
    /// Guest messages the bot hasn't seen yet (the next turn takes them all).
    #[serde(default)]
    pending: Vec<String>,
    /// A turn is queued or running for this guest.
    #[serde(default)]
    busy: bool,
    #[serde(default)]
    waiting: Option<Waiting>,
    /// The local date of the last `daily_cap` note (one a day).
    #[serde(default)]
    cap_note_day: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
    /// When the host last answered or resolved (`unread` counts guest messages after it).
    #[serde(default)]
    read_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Msg {
    id: String,
    from: String,
    text: String,
    at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Waiting {
    /// `handoff` or `daily_cap`.
    reason: String,
    why: String,
    at: DateTime<Utc>,
}

fn load(state: &Value) -> St {
    serde_json::from_value(state.clone()).unwrap_or_default()
}

fn store(state: &mut Value, s: &St) {
    if let Ok(v) = serde_json::to_value(s) {
        *state = v;
    }
}

/// The box's local date at `t` (the daily cap and `delete_on` follow the box's clock).
fn local_day(t: DateTime<Utc>) -> NaiveDate {
    t.with_timezone(&chrono::Local).date_naive()
}

/// The box's next local midnight after `t`.
pub fn next_midnight(t: DateTime<Utc>) -> DateTime<Utc> {
    let day = local_day(t) + Duration::days(1);
    day.and_hms_opt(0, 0, 0)
        .and_then(|m| m.and_local_timezone(chrono::Local).earliest())
        .map(|m| m.with_timezone(&Utc))
        .unwrap_or_else(|| t + Duration::days(1))
}

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn coded(status: u16, code: &'static str, message: impl Into<String>) -> KitError {
    KitError::Coded { status, code, message: message.into(), extra: json!({}) }
}

fn spec_str<'a>(spec: &'a Value, field: &str) -> &'a str {
    spec.get(field).and_then(Value::as_str).map(str::trim).unwrap_or("")
}

pub fn daily_cap(spec: &Value) -> u64 {
    spec.get("daily_cap").and_then(Value::as_u64).unwrap_or(DAILY_CAP).clamp(DAILY_CAP_MIN, DAILY_CAP_MAX)
}

/// The limit in force: the host's `set_cap`, else the spec's.
fn cap_now(spec: &Value, s: &St) -> u64 {
    s.cap.map_or_else(|| daily_cap(spec), |c| c.clamp(DAILY_CAP_MIN, DAILY_CAP_MAX))
}

/// A cost a turn reported, as `cost_usd` on `answer`/`handoff`; nonsense is ignored.
pub const COST_MAX_USD: f64 = 100.0;

pub fn max_guests(spec: &Value) -> usize {
    spec.get("max_guests").and_then(Value::as_u64).unwrap_or(MAX_GUESTS).clamp(1, DEFAULT_MAX_PLAYERS as u64) as usize
}

/// The house guide (the bot's facts).
pub fn guide(spec: &Value) -> &str {
    spec_str(spec, "guide")
}

/// The answering bot's name as the host knows it, from the spec (kept after the bot is deleted);
/// "The bot" when it has none.
pub fn bot_label(spec: &Value) -> String {
    let name = spec["bot"]["name"].as_str().unwrap_or("").trim();
    if name.is_empty() { "The bot".into() } else { name.to_string() }
}

fn starters(spec: &Value) -> Vec<String> {
    spec.get("starters").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).collect()).unwrap_or_default()
}

/// The bot as guests see it: a name and an avatar style, never its id.
fn bot_face(spec: &Value) -> Value {
    let name = spec.get("bot").and_then(|b| b.get("name")).and_then(Value::as_str).unwrap_or("");
    json!({ "name": name, "sprite": SPRITE })
}

impl St {
    /// Starts a new day's counts when the box's date has changed.
    fn roll(&mut self, now: DateTime<Utc>) {
        let today = local_day(now).to_string();
        if self.day != today {
            self.day = today;
            self.replies_today = 0;
            self.messages_today = 0;
        }
    }

    fn today(&self, now: DateTime<Utc>) -> (u64, u64) {
        if self.day == local_day(now).to_string() {
            (self.replies_today, self.messages_today)
        } else {
            (0, 0)
        }
    }

    fn push(&mut self, pid: &str, from: &str, text: String, at: DateTime<Utc>, client_id: Option<String>, note: Option<&str>) {
        self.next += 1;
        let id = format!("m{}", self.next);
        let g = self.guests.entry(pid.to_string()).or_default();
        g.msgs.push(Msg { id, from: from.into(), text, at, client_id, note: note.map(str::to_string) });
        if g.msgs.len() > KEEP_MESSAGES {
            let extra = g.msgs.len() - KEEP_MESSAGES;
            g.msgs.drain(..extra);
        }
    }

    /// Books what one turn cost (`body.cost_usd`, from the bot's saved reply), when it said.
    fn spend(&mut self, body: &Value) {
        if let Some(c) = body.get("cost_usd").and_then(Value::as_f64).filter(|c| c.is_finite() && (0.0..COST_MAX_USD).contains(c)) {
            self.spent_usd += c;
            self.priced_turns += 1;
        }
    }

    /// The average cost of a reply on this desk so far; `None` until a turn reported one.
    fn cost_per_reply(&self) -> Option<f64> {
        (self.priced_turns > 0).then(|| (self.spent_usd / self.priced_turns as f64 * 1e6).round() / 1e6)
    }

    /// Past the daily cap: the guest waits for the host, with one `daily_cap` note a day.
    fn cap(&mut self, pid: &str, now: DateTime<Utc>) {
        let today = local_day(now).to_string();
        let note = {
            let g = self.guests.entry(pid.to_string()).or_default();
            g.pending.clear();
            g.busy = false;
            if g.waiting.is_none() {
                g.waiting = Some(Waiting { reason: CAPPED.into(), why: CAP_WHY.into(), at: now });
            }
            if g.cap_note_day.as_deref() == Some(today.as_str()) {
                false
            } else {
                g.cap_note_day = Some(today);
                true
            }
        };
        if note {
            self.push(pid, NOTE, String::new(), now, None, Some(CAPPED));
        }
    }
}

impl G {
    fn left_this_hour(&self, now: DateTime<Utc>) -> (usize, Option<DateTime<Utc>>) {
        let recent: Vec<&DateTime<Utc>> = self.sent.iter().filter(|t| **t > now - Duration::hours(1)).collect();
        let left = PER_HOUR.saturating_sub(recent.len());
        let next = (left == 0).then(|| recent.iter().min().map(|t| **t + Duration::hours(1))).flatten();
        (left, next)
    }

    fn delete_on(&self) -> Option<String> {
        self.last_at.map(|t| (local_day(t) + Duration::days(KEEP_DAYS)).to_string())
    }

    fn unread(&self) -> usize {
        self.msgs.iter().filter(|m| m.from == GUEST && self.read_at.is_none_or(|r| m.at > r)).count()
    }

    fn handoff_reason(&self) -> Option<&str> {
        self.waiting.as_ref().map(|w| w.why.as_str())
    }
}

fn messages(g: Option<&G>, with_client_ids: bool) -> Vec<Value> {
    g.map(|g| {
        g.msgs
            .iter()
            .map(|m| {
                let mut v = json!({ "id": m.id, "from": m.from, "text": m.text, "at": m.at });
                if let Some(n) = &m.note {
                    v["note"] = json!(n);
                }
                if with_client_ids {
                    if let Some(c) = &m.client_id {
                        v["client_id"] = json!(c);
                    }
                }
                v
            })
            .collect()
    })
    .unwrap_or_default()
}

fn guest_from<'a>(ctx: &'a KitCtx, body: &Value) -> Result<&'a Player, KitError> {
    let id = body.get("guest").and_then(Value::as_str).unwrap_or("");
    ctx.player(id).ok_or_else(|| coded(404, "no_such_player", "That guest isn't in this pocket."))
}

fn host_text(body: &Value) -> Result<String, KitError> {
    let text = body.get("text").and_then(Value::as_str).unwrap_or("").trim();
    if text.is_empty() {
        return Err(coded(400, "empty", "Write a reply first."));
    }
    if text.chars().count() > REPLY_MAX {
        return Err(coded(413, "too_long", format!("A reply can have up to {REPLY_MAX} characters.")));
    }
    Ok(text.to_string())
}

pub struct Desk;

impl Desk {
    fn guest_row(s: &St, p: &Player) -> Value {
        let g = s.guests.get(&p.id);
        let last = g.and_then(|g| g.msgs.iter().rev().find(|m| m.from != NOTE)).map(|m| json!({ "from": m.from, "text": cut(&m.text, LAST_MAX), "at": m.at }));
        json!({
            "id": p.id,
            "name": p.name,
            "creature": p.avatar,
            "waiting": g.is_some_and(|g| g.waiting.is_some()),
            "handoff_reason": g.and_then(G::handoff_reason),
            "last": last,
            "unread": g.map_or(0, G::unread),
            "msgs": g.map_or(0, |g| g.msgs.iter().filter(|m| m.from != NOTE).count()),
            "delete_on": g.and_then(G::delete_on),
        })
    }

    /// The host's guest list, newest activity first.
    fn guest_list(ctx: &KitCtx, s: &St) -> Vec<Value> {
        let mut rows: Vec<(DateTime<Utc>, Value)> = ctx
            .players
            .iter()
            .map(|p| {
                let at = s.guests.get(&p.id).and_then(|g| g.msgs.last().map(|m| m.at)).unwrap_or(p.joined_at);
                (at, Self::guest_row(s, p))
            })
            .collect();
        rows.sort_by_key(|a| std::cmp::Reverse(a.0));
        rows.into_iter().map(|(_, v)| v).collect()
    }

    fn face(spec: &Value) -> Value {
        json!({
            "kit": ID,
            "title": spec_str(spec, "title"),
            "place": spec_str(spec, "place"),
            "host_name": spec_str(spec, "host_name"),
            "bot": bot_face(spec),
            "keep_days": KEEP_DAYS,
        })
    }

    fn guest_view(ctx: &KitCtx, s: &St, me: &str) -> Value {
        let g = s.guests.get(me);
        let player = ctx.player(me);
        let (left, next_at) = g.map_or((PER_HOUR, None), |g| g.left_this_hour(ctx.now));
        let mut v = Self::face(ctx.spec);
        v["me"] = json!({
            "id": me,
            "name": player.map(|p| p.name.as_str()),
            "creature": player.map(|p| p.avatar.as_str()),
            "delete_on": g.and_then(G::delete_on),
        });
        v["starters"] = json!(starters(ctx.spec));
        v["messages"] = json!(messages(g, true));
        v["waiting_for_host"] = json!(g.is_some_and(|g| g.waiting.is_some()));
        v["bot_typing"] = json!(g.is_some_and(|g| g.busy));
        v["limits"] = json!({ "per_hour": PER_HOUR, "left_this_hour": left, "next_at": next_at, "max_chars": MAX_CHARS });
        v
    }

    fn say(ctx: &mut KitCtx, s: &mut St, pid: &str, body: &Value) -> Result<(), KitError> {
        let text = body.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if text.is_empty() {
            return Err(coded(400, "empty", "Write a message first."));
        }
        if text.chars().count() > MAX_CHARS {
            return Err(coded(413, "too_long", format!("A message can have up to {MAX_CHARS} characters.")));
        }
        let client_id = body.get("client_id").and_then(Value::as_str).map(|c| cut(c.trim(), 64)).filter(|c| !c.is_empty());
        let now = ctx.now;
        let g = s.guests.entry(pid.to_string()).or_default();
        g.sent.retain(|t| *t > now - Duration::hours(1));
        let (left, next_at) = g.left_this_hour(now);
        if left == 0 {
            let retry_at = next_at.unwrap_or(now + Duration::hours(1));
            return Err(KitError::Coded {
                status: 429,
                code: "slow_down",
                message: format!("That's {PER_HOUR} messages this hour. You can write again soon."),
                extra: json!({ "retry_at": retry_at }),
            });
        }
        g.sent.push(now);
        g.last_at = Some(now);
        s.roll(now);
        s.messages_today += 1;
        s.push(pid, GUEST, text.clone(), now, client_id, None);
        if s.replies_today >= cap_now(ctx.spec, s) {
            s.cap(pid, now);
        } else {
            let g = s.guests.entry(pid.to_string()).or_default();
            g.pending.push(text);
            g.busy = true;
        }
        Ok(())
    }
}

impl Kit for Desk {
    fn id(&self) -> &'static str {
        ID
    }

    fn rules(&self, spec: &Value) -> KitRules {
        KitRules {
            retention: Retention::AfterLastMessage { days: KEEP_DAYS },
            needs_pin: spec.get("needs_pin").and_then(Value::as_bool).unwrap_or(true),
            // The kit's own `max_guests` is checked in `admit`, so a full desk says `guest_max`
            max_players: DEFAULT_MAX_PLAYERS,
            guest_actions: &["say"],
            host_actions: &["reply", "resolve", "set_cap"],
        }
    }

    fn validate_spec(&self, spec: &Value) -> Result<(), String> {
        if !spec.is_object() {
            return Err("The spec must be an object.".into());
        }
        let title = spec_str(spec, "title");
        if title.is_empty() || title.chars().count() > 80 {
            return Err("Give the front desk a title of up to 80 characters.".into());
        }
        for (field, max) in [("place", 60), ("host_name", 40)] {
            match spec.get(field) {
                None | Some(Value::Null) => {}
                Some(Value::String(s)) if s.trim().chars().count() <= max => {}
                _ => return Err(format!("{field} must be text of up to {max} characters.")),
            }
        }
        let g = guide(spec);
        if g.is_empty() {
            return Err("Write the house guide: the facts the bot answers from.".into());
        }
        if g.chars().count() > GUIDE_MAX {
            return Err(format!("The house guide can have up to {GUIDE_MAX} characters."));
        }
        if let Some(c) = spec.get("daily_cap").filter(|v| !v.is_null()) {
            if !c.as_u64().is_some_and(|c| (DAILY_CAP_MIN..=DAILY_CAP_MAX).contains(&c)) {
                return Err(format!("daily_cap must be a whole number from {DAILY_CAP_MIN} to {DAILY_CAP_MAX}."));
            }
        }
        if let Some(m) = spec.get("max_guests").filter(|v| !v.is_null()) {
            if !m.as_u64().is_some_and(|m| (1..=DEFAULT_MAX_PLAYERS as u64).contains(&m)) {
                return Err(format!("max_guests must be a whole number from 1 to {DEFAULT_MAX_PLAYERS}."));
            }
        }
        if let Some(st) = spec.get("starters").filter(|v| !v.is_null()) {
            let ok = st.as_array().is_some_and(|a| {
                a.len() <= MAX_STARTERS && a.iter().all(|s| s.as_str().is_some_and(|s| !s.trim().is_empty() && s.trim().chars().count() <= STARTER_MAX))
            });
            if !ok {
                return Err(format!("starters: up to {MAX_STARTERS} short questions of up to {STARTER_MAX} characters each."));
            }
        }
        if spec.get("needs_pin").is_some_and(|v| !v.is_null() && !v.is_boolean()) {
            return Err("needs_pin must be true or false.".into());
        }
        if spec.get("answer_bot").is_some_and(|v| !v.is_null() && !v.is_string()) {
            return Err("answer_bot must be a bot's id.".into());
        }
        Ok(())
    }

    fn init(&self, _spec: &Value) -> Value {
        serde_json::to_value(St { v: 1, ..St::default() }).unwrap_or(Value::Null)
    }

    fn admit(&self, ctx: &KitCtx, _state: &Value, _name_key: &str) -> Result<(), KitError> {
        let max = max_guests(ctx.spec);
        if ctx.players.len() >= max {
            return Err(KitError::Coded { status: 409, code: "guest_max", message: "This guest page is full.".into(), extra: json!({ "max_guests": max }) });
        }
        Ok(())
    }

    fn on_remove(&self, _ctx: &mut KitCtx, state: &mut Value, player_id: &str) {
        let mut s = load(state);
        s.guests.remove(player_id);
        store(state, &s);
    }

    fn act(&self, ctx: &mut KitCtx, state: &mut Value, actor: &Viewer, name: &str, body: &Value) -> Result<(), KitError> {
        let mut s = load(state);
        match (actor, name) {
            (Viewer::Guest(pid), "say") => Self::say(ctx, &mut s, pid, body)?,
            (Viewer::Host, "reply") => {
                let pid = guest_from(ctx, body)?.id.clone();
                let text = host_text(body)?;
                s.push(&pid, HOST, text, ctx.now, None, None);
                let g = s.guests.entry(pid).or_default();
                g.waiting = None;
                g.read_at = Some(ctx.now);
            }
            (Viewer::Host, "resolve") => {
                let pid = guest_from(ctx, body)?.id.clone();
                let g = s.guests.entry(pid).or_default();
                g.waiting = None;
                g.read_at = Some(ctx.now);
            }
            // The card's cap stepper: clamped to 10..200. Guests already waiting past the old
            // limit stay with the host; their next message goes to the bot while under it.
            (Viewer::Host, "set_cap") => {
                let cap = body.get("cap").and_then(|c| c.as_i64().or_else(|| c.as_f64().filter(|f| f.is_finite() && f.fract() == 0.0).map(|f| f as i64)));
                let Some(cap) = cap else {
                    return Err(KitError::BadInput(format!("cap must be a whole number from {DAILY_CAP_MIN} to {DAILY_CAP_MAX}.")));
                };
                s.cap = Some(cap.clamp(DAILY_CAP_MIN as i64, DAILY_CAP_MAX as i64) as u64);
            }
            _ => return Err(KitError::BadInput("That action isn't one this pocket knows.".into())),
        }
        store(state, &s);
        Ok(())
    }

    fn stable_address_only(&self) -> bool {
        true
    }

    fn view(&self, ctx: &KitCtx, state: &Value, viewer: &Viewer) -> Value {
        let s = load(state);
        match viewer {
            Viewer::Guest(id) => Self::guest_view(ctx, &s, id),
            Viewer::Host => {
                let mut v = Self::face(ctx.spec);
                v["guests"] = json!(Self::guest_list(ctx, &s));
                v
            }
            // A TV never shows a guest's conversation
            Viewer::BigScreen => Self::face(ctx.spec),
        }
    }

    fn lobby(&self, ctx: &KitCtx, _state: &Value) -> Value {
        Self::face(ctx.spec)
    }

    fn host_summary(&self, ctx: &KitCtx, state: &Value) -> Value {
        let s = load(state);
        let (replies, msgs) = s.today(ctx.now);
        let g = guide(ctx.spec);
        let waiting = ctx.players.iter().filter(|p| s.guests.get(&p.id).is_some_and(|g| g.waiting.is_some())).count();
        json!({
            "counts": { "guests": ctx.players.len(), "waiting": waiting, "replies_today": replies, "daily_cap": cap_now(ctx.spec, &s), "messages_today": msgs },
            "cap_resets_at": next_midnight(ctx.now),
            // The core fills the bot's name and model and the address from what it knows
            "answer_bot": { "id": spec_str(ctx.spec, "answer_bot"), "name": bot_face(ctx.spec)["name"], "sprite": SPRITE, "model": null },
            // Measured on this desk: the average of its turns' reported costs (null until one)
            "cost_per_reply_usd": s.cost_per_reply(),
            "guide_chars": g.chars().count(),
            "guide_facts": g.lines().filter(|l| !l.trim().is_empty()).count(),
            "address": null,
            "guests": Self::guest_list(ctx, &s),
        })
    }

    fn answer_bot(&self) -> bool {
        true
    }

    fn private_guests(&self) -> bool {
        true
    }

    fn server_event(&self, ctx: &mut KitCtx, state: &mut Value, name: &str, body: &Value) -> Result<Value, KitError> {
        let mut s = load(state);
        let pid = guest_from(ctx, body)?.id.clone();
        let out = match name {
            // The next turn's messages (all that came since the last one), or none: then the
            // worker stops. Each turn taken counts against the daily cap.
            "take" => {
                s.roll(ctx.now);
                let cap = cap_now(ctx.spec, &s);
                let g = s.guests.entry(pid.clone()).or_default();
                if g.pending.is_empty() {
                    g.busy = false;
                    json!({ "texts": [] })
                } else if s.replies_today >= cap {
                    s.cap(&pid, ctx.now);
                    json!({ "texts": [], "capped": true })
                } else {
                    let texts = std::mem::take(&mut g.pending);
                    let thread = g.thread_id.clone();
                    s.replies_today += 1;
                    json!({ "texts": texts, "thread_id": thread })
                }
            }
            "thread" => {
                // The first thread wins: a second one opened at the same time is the caller's to drop
                let t = body.get("thread_id").and_then(Value::as_str).unwrap_or("");
                let g = s.guests.entry(pid).or_default();
                if g.thread_id.is_none() {
                    g.thread_id = Some(t.to_string()).filter(|t| !t.is_empty());
                }
                json!({ "thread_id": g.thread_id })
            }
            "answer" => {
                s.spend(body);
                let text = cut(body.get("text").and_then(Value::as_str).unwrap_or("").trim(), REPLY_MAX);
                if !text.is_empty() {
                    s.push(&pid, BOT, text, ctx.now, None, None);
                }
                json!({})
            }
            "handoff" => {
                s.spend(body);
                let why = cut(body.get("why").and_then(Value::as_str).unwrap_or("").trim(), REASON_MAX);
                let note = if body.get("note").and_then(Value::as_str) == Some(HOST_AWAY) { HOST_AWAY } else { HANDOFF };
                s.push(&pid, NOTE, String::new(), ctx.now, None, Some(note));
                s.guests.entry(pid).or_default().waiting = Some(Waiting { reason: HANDOFF.into(), why, at: ctx.now });
                json!({})
            }
            // The worker couldn't go on (its bot is gone): nothing is running for this guest
            "idle" => {
                let g = s.guests.entry(pid).or_default();
                g.busy = false;
                g.pending.clear();
                json!({})
            }
            _ => return Err(KitError::BadInput("unknown server event".into())),
        };
        store(state, &s);
        Ok(out)
    }

    fn owned_threads(&self, state: &Value) -> Vec<String> {
        load(state).guests.values().filter_map(|g| g.thread_id.clone()).collect()
    }

    fn idle_players(&self, ctx: &KitCtx, state: &Value) -> Vec<String> {
        let s = load(state);
        ctx.players
            .iter()
            .filter(|p| {
                let last = s.guests.get(&p.id).and_then(|g| g.last_at).unwrap_or(p.joined_at);
                last + Duration::days(KEEP_DAYS) <= ctx.now
            })
            .map(|p| p.id.clone())
            .collect()
    }

    fn host_detail(&self, ctx: &KitCtx, state: &Value, key: &str) -> Option<Value> {
        let id = key.strip_prefix("guest:")?;
        let p = ctx.player(id)?;
        let s = load(state);
        let g = s.guests.get(id);
        Some(json!({
            "guest": {
                "id": p.id,
                "name": p.name,
                "creature": p.avatar,
                "delete_on": g.and_then(G::delete_on),
                "waiting": g.is_some_and(|g| g.waiting.is_some()),
                "handoff_reason": g.and_then(G::handoff_reason),
            },
            "messages": messages(g, false),
        }))
    }
}

/// For the runtime: the guests with a turn queued or running (re-kicked after a restart).
pub fn busy_guests(state: &Value) -> Vec<String> {
    load(state).guests.into_iter().filter(|(_, g)| g.busy).map(|(id, _)| id).collect()
}

/// One row of the Needs-you list for a desk pocket.
#[derive(Debug, Clone)]
pub struct WaitingGuest {
    pub guest: String,
    /// The reason label shown to the host.
    pub why: String,
    /// The guest's last words, at most 200 characters.
    pub quote: String,
    /// Since when the guest has waited.
    pub at: DateTime<Utc>,
    pub thread: Option<String>,
}

/// For the Needs-you list: each guest waiting for the host.
pub fn waiting_guests(state: &Value) -> Vec<WaitingGuest> {
    load(state)
        .guests
        .into_iter()
        .filter_map(|(id, g)| {
            let w = g.waiting.as_ref()?;
            let quote = g.msgs.iter().rev().find(|m| m.from == GUEST).map(|m| cut(&m.text, 200)).unwrap_or_default();
            Some(WaitingGuest { guest: id, why: w.why.clone(), quote, at: w.at, thread: g.thread_id.clone() })
        })
        .collect()
}

/// `true` when this guest's wait for the host began with their message at `at` (a `say` that
/// went over the daily cap): the host is told once.
pub fn capped_at(state: &Value, guest: &str, at: DateTime<Utc>) -> bool {
    load(state).guests.get(guest).and_then(|g| g.waiting.as_ref()).is_some_and(|w| w.reason == CAPPED && w.at == at)
}

/// The bot's thread with this guest, once it has one.
pub fn thread_of(state: &Value, guest: &str) -> Option<String> {
    load(state).guests.get(guest).and_then(|g| g.thread_id.clone())
}

/// The guest's latest message time (for [`capped_at`]).
pub fn last_message_at(state: &Value, guest: &str) -> Option<DateTime<Utc>> {
    load(state).guests.get(guest).and_then(|g| g.last_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Value {
        json!({ "title": "Sea-view flat", "place": "Piran", "host_name": "Emma", "guide": "Wi-Fi: sea123\nCheck-out: 10 AM", "daily_cap": 10, "max_guests": 2, "bot": { "name": "Bell" } })
    }

    fn players(now: DateTime<Utc>) -> Vec<Player> {
        ["ann", "ben"].iter().map(|n| Player { id: format!("pl_{n}"), name: n.to_string(), avatar: "fox".into(), team: None, joined_at: now }).collect()
    }

    fn say(st: &mut Value, ctx: &mut KitCtx, who: &str, text: &str) -> Result<(), KitError> {
        Desk.act(ctx, st, &Viewer::Guest(who.into()), "say", &json!({ "text": text, "client_id": "c1" }))
    }

    #[test]
    fn specs_are_checked() {
        assert!(Desk.validate_spec(&spec()).is_ok());
        for bad in [
            json!({ "title": "x" }),
            json!({ "title": "x", "guide": "g", "daily_cap": 5 }),
            json!({ "title": "x", "guide": "g", "max_guests": 61 }),
            json!({ "title": "x", "guide": "g", "starters": ["a", "b", "c", "d"] }),
            json!({ "title": "x", "guide": "g", "starters": ["this one is much too long to fit"] }),
            json!({ "title": "x", "guide": "g".repeat(GUIDE_MAX + 1) }),
        ] {
            assert!(Desk.validate_spec(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_guest_sees_only_their_own_conversation() {
        let now = Utc::now();
        let ps = players(now);
        let spec = spec();
        let mut st = Desk.init(&spec);
        let mut ctx = KitCtx::new(&spec, &ps, now);
        say(&mut st, &mut ctx, "pl_ann", "Where do I park? ann-secret").unwrap();
        say(&mut st, &mut ctx, "pl_ben", "Wi-Fi please ben-secret").unwrap();
        let ann = Desk.view(&ctx, &st, &Viewer::Guest("pl_ann".into())).to_string();
        assert!(ann.contains("ann-secret") && !ann.contains("ben-secret") && !ann.contains("\"ben\""), "{ann}");
        for public in [Desk.lobby(&ctx, &st), Desk.view(&ctx, &st, &Viewer::BigScreen)] {
            let t = public.to_string();
            assert!(!t.contains("secret") && !t.contains("\"ann\"") && !t.contains("pl_"), "{t}");
        }
        let host = Desk.host_summary(&ctx, &st).to_string();
        assert!(host.contains("ann-secret") || host.contains("Where do I park"), "the host sees their guests");
    }

    #[test]
    fn the_twenty_first_message_in_an_hour_waits() {
        let now = Utc::now();
        let ps = players(now);
        let spec = json!({ "title": "x", "guide": "g", "daily_cap": 200 });
        let mut st = Desk.init(&spec);
        let mut ctx = KitCtx::new(&spec, &ps, now);
        for i in 0..PER_HOUR {
            say(&mut st, &mut ctx, "pl_ann", &format!("q{i}")).unwrap();
        }
        match say(&mut st, &mut ctx, "pl_ann", "one more") {
            Err(KitError::Coded { status: 429, code: "slow_down", extra, .. }) => assert!(extra["retry_at"].is_string()),
            other => panic!("{other:?}"),
        }
        let v = Desk.view(&ctx, &st, &Viewer::Guest("pl_ann".into()));
        assert_eq!(v["limits"]["left_this_hour"], json!(0));
        assert!(v["limits"]["next_at"].is_string());
        // Ben has his own hour
        say(&mut st, &mut ctx, "pl_ben", "hi").unwrap();
        // An hour later Ann may write again
        let later = now + Duration::minutes(61);
        let mut ctx = KitCtx::new(&spec, &ps, later);
        say(&mut st, &mut ctx, "pl_ann", "back").unwrap();
        assert!(matches!(say(&mut st, &mut ctx, "pl_ann", &"x".repeat(MAX_CHARS + 1)), Err(KitError::Coded { status: 413, .. })));
    }

    #[test]
    fn turns_count_against_the_daily_cap_and_then_messages_wait_for_the_host() {
        let now = Utc::now();
        let ps = players(now);
        let spec = spec();
        let mut st = Desk.init(&spec);
        let mut ctx = KitCtx::new(&spec, &ps, now);
        for i in 0..10 {
            say(&mut st, &mut ctx, "pl_ann", &format!("q{i}")).unwrap();
            let t = Desk.server_event(&mut ctx, &mut st, "take", &json!({ "guest": "pl_ann" })).unwrap();
            assert_eq!(t["texts"], json!([format!("q{i}")]));
        }
        say(&mut st, &mut ctx, "pl_ann", "q10").unwrap();
        assert!(capped_at(&st, "pl_ann", now));
        say(&mut st, &mut ctx, "pl_ann", "q11").unwrap();
        let v = Desk.view(&ctx, &st, &Viewer::Guest("pl_ann".into()));
        assert_eq!(v["waiting_for_host"], json!(true));
        assert_eq!(v["bot_typing"], json!(false));
        let notes: Vec<&Value> = v["messages"].as_array().unwrap().iter().filter(|m| m["note"] == json!(CAPPED)).collect();
        assert_eq!(notes.len(), 1, "one daily_cap note a day");
        assert_eq!(Desk.server_event(&mut ctx, &mut st, "take", &json!({ "guest": "pl_ann" })).unwrap()["texts"], json!([]));
        assert_eq!(waiting_guests(&st).len(), 1);
        // The host answers: Ann no longer waits, and Ben never saw any of it
        Desk.act(&mut ctx, &mut st, &Viewer::Host, "reply", &json!({ "guest": "pl_ann", "text": "Sure, 2 PM." })).unwrap();
        assert!(waiting_guests(&st).is_empty());
        let ben = Desk.view(&ctx, &st, &Viewer::Guest("pl_ben".into()));
        assert_eq!(ben["messages"], json!([]));
        // A new day starts with a fresh cap
        let s = Desk.host_summary(&KitCtx::new(&spec, &ps, now + Duration::days(1)), &st);
        assert_eq!(s["counts"]["replies_today"], json!(0));
    }

    #[test]
    fn idle_guests_go_after_seven_days() {
        let now = Utc::now();
        let ps = players(now);
        let spec = spec();
        let mut st = Desk.init(&spec);
        let mut ctx = KitCtx::new(&spec, &ps, now);
        say(&mut st, &mut ctx, "pl_ann", "hi").unwrap();
        let mut ctx = KitCtx::new(&spec, &ps, now + Duration::days(3));
        say(&mut st, &mut ctx, "pl_ben", "hi").unwrap();
        let ctx = KitCtx::new(&spec, &ps, now + Duration::days(7) + Duration::minutes(1));
        assert_eq!(Desk.idle_players(&ctx, &st), vec!["pl_ann".to_string()]);
        Desk.server_event(&mut KitCtx::new(&spec, &ps, now), &mut st, "thread", &json!({ "guest": "pl_ann", "thread_id": "th1" })).unwrap();
        assert_eq!(Desk.owned_threads(&st), vec!["th1".to_string()]);
        Desk.on_remove(&mut KitCtx::new(&spec, &ps, now), &mut st, "pl_ann");
        assert!(Desk.owned_threads(&st).is_empty());
    }

    #[test]
    fn a_hand_off_with_no_reply_says_host_away_and_the_first_thread_wins() {
        let now = Utc::now();
        let ps = players(now);
        let spec = spec();
        let mut st = Desk.init(&spec);
        let mut ctx = KitCtx::new(&spec, &ps, now);
        say(&mut st, &mut ctx, "pl_ann", "hi").unwrap();
        Desk.server_event(&mut ctx, &mut st, "handoff", &json!({ "guest": "pl_ann", "why": "Bell had no reply it could send", "note": HOST_AWAY })).unwrap();
        say(&mut st, &mut ctx, "pl_ben", "hi").unwrap();
        Desk.server_event(&mut ctx, &mut st, "handoff", &json!({ "guest": "pl_ben", "why": "Late check-out" })).unwrap();
        let note = |who: &str| {
            let v = Desk.view(&ctx, &st, &Viewer::Guest(who.into()));
            v["messages"].as_array().unwrap().iter().filter_map(|m| m["note"].as_str().map(str::to_string)).collect::<Vec<_>>()
        };
        assert_eq!(note("pl_ann"), vec![HOST_AWAY.to_string()]);
        assert_eq!(note("pl_ben"), vec![HANDOFF.to_string()]);
        let whys: Vec<String> = waiting_guests(&st).into_iter().map(|w| w.why).collect();
        assert!(whys.contains(&"Bell had no reply it could send".to_string()), "{whys:?}");
        assert_eq!(bot_label(&spec), "Bell");

        let t = Desk.server_event(&mut ctx, &mut st, "thread", &json!({ "guest": "pl_ann", "thread_id": "th1" })).unwrap();
        assert_eq!(t["thread_id"], json!("th1"));
        let t = Desk.server_event(&mut ctx, &mut st, "thread", &json!({ "guest": "pl_ann", "thread_id": "th2" })).unwrap();
        assert_eq!(t["thread_id"], json!("th1"), "the second opener drops its own");
        assert_eq!(thread_of(&st, "pl_ann").as_deref(), Some("th1"));
    }

    #[test]
    fn the_hosts_set_cap_wins_over_the_spec_and_is_clamped() {
        let now = Utc::now();
        let ps = players(now);
        let spec = spec(); // daily_cap 10
        let mut st = Desk.init(&spec);
        let mut ctx = KitCtx::new(&spec, &ps, now);
        assert!(Desk.rules(&spec).host_actions.contains(&"set_cap"));
        for i in 0..10 {
            say(&mut st, &mut ctx, "pl_ann", &format!("q{i}")).unwrap();
            Desk.server_event(&mut ctx, &mut st, "take", &json!({ "guest": "pl_ann" })).unwrap();
        }
        // At the spec's 10: the next one waits for the host
        say(&mut st, &mut ctx, "pl_ann", "q10").unwrap();
        assert!(capped_at(&st, "pl_ann", now));
        // The host raises it to 20: Ann still waits (the host has her), Ben's message goes to the bot
        Desk.act(&mut ctx, &mut st, &Viewer::Host, "set_cap", &json!({ "cap": 20 })).unwrap();
        assert_eq!(Desk.host_summary(&ctx, &st)["counts"]["daily_cap"], json!(20));
        assert_eq!(waiting_guests(&st).len(), 1);
        say(&mut st, &mut ctx, "pl_ben", "Wi-Fi?").unwrap();
        let t = Desk.server_event(&mut ctx, &mut st, "take", &json!({ "guest": "pl_ben" })).unwrap();
        assert_eq!(t["texts"], json!(["Wi-Fi?"]));
        // Lowered under today's replies: the next turn waits again
        Desk.act(&mut ctx, &mut st, &Viewer::Host, "set_cap", &json!({ "cap": 10 })).unwrap();
        say(&mut st, &mut ctx, "pl_ben", "And parking?").unwrap();
        assert_eq!(Desk.view(&ctx, &st, &Viewer::Guest("pl_ben".into()))["waiting_for_host"], json!(true));
        // Clamped to 10..200; a whole float is fine; anything else is refused and changes nothing
        for (cap, want) in [(json!(1), 10), (json!(500), 200), (json!(-3), 10), (json!(60.0), 60)] {
            Desk.act(&mut ctx, &mut st, &Viewer::Host, "set_cap", &json!({ "cap": cap })).unwrap();
            assert_eq!(Desk.host_summary(&ctx, &st)["counts"]["daily_cap"], json!(want), "{cap}");
        }
        for bad in [json!({}), json!({ "cap": "50" }), json!({ "cap": 12.5 }), json!({ "cap": null })] {
            assert!(matches!(Desk.act(&mut ctx, &mut st, &Viewer::Host, "set_cap", &bad), Err(KitError::BadInput(_))), "{bad}");
        }
        assert_eq!(Desk.host_summary(&ctx, &st)["counts"]["daily_cap"], json!(60));
        // A guest can't set it
        assert!(Desk.act(&mut ctx, &mut st, &Viewer::Guest("pl_ann".into()), "set_cap", &json!({ "cap": 200 })).is_err());
    }

    #[test]
    fn cost_per_reply_is_the_average_of_the_turns_that_reported_one() {
        let now = Utc::now();
        let ps = players(now);
        let spec = spec();
        let mut st = Desk.init(&spec);
        let mut ctx = KitCtx::new(&spec, &ps, now);
        assert_eq!(Desk.host_summary(&ctx, &st)["cost_per_reply_usd"], Value::Null, "null until a turn says");
        // An engine that doesn't report a cost books nothing
        Desk.server_event(&mut ctx, &mut st, "answer", &json!({ "guest": "pl_ann", "text": "a", "cost_usd": null })).unwrap();
        assert_eq!(Desk.host_summary(&ctx, &st)["cost_per_reply_usd"], Value::Null);
        Desk.server_event(&mut ctx, &mut st, "answer", &json!({ "guest": "pl_ann", "text": "b", "cost_usd": 0.004 })).unwrap();
        Desk.server_event(&mut ctx, &mut st, "handoff", &json!({ "guest": "pl_ben", "why": "Late check-out", "cost_usd": 0.002 })).unwrap();
        // Nonsense is ignored
        for bad in [json!(-1.0), json!(1e9), json!("0.5")] {
            Desk.server_event(&mut ctx, &mut st, "answer", &json!({ "guest": "pl_ann", "text": "c", "cost_usd": bad })).unwrap();
        }
        assert_eq!(Desk.host_summary(&ctx, &st)["cost_per_reply_usd"], json!(0.003));
        // It survives a save and load
        let again: St = serde_json::from_value(st.clone()).unwrap();
        assert_eq!(again.priced_turns, 2);
    }

    #[test]
    fn a_full_desk_says_guest_max() {
        let now = Utc::now();
        let ps = players(now);
        let spec = spec();
        let ctx = KitCtx::new(&spec, &ps, now);
        assert!(matches!(Desk.admit(&ctx, &Desk.init(&spec), "cy"), Err(KitError::Coded { code: "guest_max", .. })));
    }
}
