//! pa14b: the Wall's best 30, picked the morning after it closes. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa14/pa14b-brief.md, API in
//! pa14b-impl.md next to it ("Pocket API v1: wall, pa14b").
//!
//! On the pocket supervisor's minute tick ([`super::schedule::run_once`]), each wall whose pick is
//! due (the first 09:00 on the person's clock after uploads stopped) gets one step, under the
//! pocket's lock:
//! - the host already chose a best by hand: no pick, ever (never overridden);
//! - 30 or fewer approved photos: all of them are the proposal, no bot call;
//! - more: the wall's bot gets one turn. The state says `picking` (saved before the turn starts,
//!   so a second tick never starts another), the turn may read small previews of the approved
//!   photos with [`TOOL_PHOTOS`] (made here in memory from the files on this box; nothing but the
//!   bot's own model call sees them) and answers with [`TOOL_PROPOSE`]. When the turn ends without
//!   a proposal (it failed, the bot answered nothing usable), `picking` goes back to false, with no
//!   proposal, and the host stars by hand as before. A pick a restart cut off runs once more on the
//!   next tick, then never ([`wall::PICK_TRIES`]).
//!
//! Guests see nothing of it until the host's `set_best`; that the bot picked the best shown
//! (`best_by: "snap"`) needs the bot's own proposal unchanged. "30 or fewer, so all" is never
//! the bot's: nothing picked those.

use super::kits::{spec_date, KitCtx};
use super::schedule::{self, Origin, Zone};
use super::{wall, PResult, Snapshot};
use crate::db::models::{Kwant, MessageExtra, NewMessage};
use crate::server::AppState;
use base64::Engine;
use chrono::{DateTime, NaiveTime, Utc};
use serde_json::{json, Value};

pub const TOOL_PHOTOS: &str = "pocket_wall_photos";
pub const TOOL_PROPOSE: &str = "pocket_wall_propose_best";
/// The pick runs at this hour on the person's clock, the morning after uploads stopped.
pub const PICK_HOUR: u32 = 9;
/// Previews the bot may see at most (an evenly spaced sample of a bigger wall): one turn's images
/// stay well under the model's per-request limits (about 100 images, 32 MB).
pub const SHOW_MAX: usize = 90;
/// Previews per [`TOOL_PHOTOS`] page.
pub const PAGE: usize = 10;
/// A preview's longest side, in pixels.
pub const THUMB_EDGE: u32 = 512;
const THUMB_QUALITY: u8 = 70;
/// A photo bigger than this (either side) or needing more memory to decode gets no preview.
const DECODE_MAX_EDGE: u32 = 12_000;
const DECODE_MAX_ALLOC: u64 = 256 * 1024 * 1024;
/// One preview is made at a time across every wall (a decoded 48 MP photo takes about 200 MB).
static PREVIEWS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
/// Who the pick's prompt is from in the chat, and the turn's label.
pub const LABEL: &str = "Wall: best 30";

// ---- When ----

/// When uploads stopped: the spec's `closes_at` or the pocket's end, whichever came first.
/// `None`: neither happened yet, or the wall has no `closes_at` and is still open (no pick).
pub fn stopped_at(s: &Snapshot) -> Option<DateTime<Utc>> {
    let closes = s.spec.get("closes_at").and_then(spec_date);
    match (closes, s.row.ended_at) {
        (Some(c), Some(e)) => Some(c.min(e)),
        (c, e) => c.or(e),
    }
}

/// The first [`PICK_HOUR`]:00 on the wall clock of `zone` after `stopped`.
pub fn pick_at(stopped: DateTime<Utc>, zone: &Zone) -> DateTime<Utc> {
    let hour = NaiveTime::from_hms_opt(PICK_HOUR, 0, 0).expect("a valid hour");
    let day = zone.local(stopped).date();
    let at = zone.instant(day, hour);
    if at > stopped {
        return at;
    }
    zone.instant(day.succ_opt().unwrap_or(day), hour)
}

// ---- Picks running in this process ----

fn register(state: &AppState, code: &str, kwant_id: &str) {
    state.pocket.picks.lock().unwrap_or_else(|e| e.into_inner()).insert(code.to_string(), kwant_id.to_string());
}

fn unregister(state: &AppState, code: &str) {
    state.pocket.picks.lock().unwrap_or_else(|e| e.into_inner()).remove(code);
}

fn running_here(state: &AppState, code: &str) -> bool {
    state.pocket.picks.lock().unwrap_or_else(|e| e.into_inner()).contains_key(code)
}

fn running_for(state: &AppState, code: &str, kwant_id: &str) -> bool {
    state.pocket.picks.lock().unwrap_or_else(|e| e.into_inner()).get(code).is_some_and(|k| k == kwant_id)
}

/// `kwant_id` is picking a wall's best now: its turns are offered the two pick tools.
pub fn picking_bot(state: &AppState, kwant_id: &str) -> bool {
    state.pocket.picks.lock().unwrap_or_else(|e| e.into_inner()).values().any(|k| k == kwant_id)
}

// ---- The tick ----

/// One step for every wall: the codes whose pick started a bot turn or proposed all photos.
pub async fn run_due(state: &AppState, now: DateTime<Utc>) -> Vec<String> {
    let codes = match state.db.pocket_codes_of_kit(wall::ID).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("pocket apps: couldn't list walls for the best-30 pick: {e}");
            return Vec::new();
        }
    };
    let mut ran = Vec::new();
    for code in codes {
        match step(state, &code, now).await {
            Ok(true) => ran.push(code),
            Ok(false) => {}
            Err(e) => tracing::warn!(code = %code, "pocket apps: the wall's best-30 pick failed: {}", e.code),
        }
    }
    ran
}

/// The chat the pick's turn and lines go to: the pocket's own, else its owner bot's direct chat.
fn origin_for(s: &Snapshot, sched: Option<&crate::db::pocket::ScheduleRow>) -> Option<Origin> {
    sched.and_then(schedule::origin_of).or_else(|| s.row.owner_kwant_id.clone().map(|kwant_id| Origin { kwant_id, channel_id: None, thread_id: None }))
}

async fn step(state: &AppState, code: &str, now: DateTime<Utc>) -> PResult<bool> {
    let lock = state.pocket.lock_for(code);
    let guard = lock.lock().await;
    let Some(s) = super::snapshot(state, code).await? else { return Ok(false) };
    if s.row.kit != wall::ID || wall::pick_settled(&s.kit_state) {
        return Ok(false);
    }
    let sched = state.db.pocket_schedule(code).await?;
    let zone = Zone::parse(sched.as_ref().map_or("UTC", |r| r.zone.as_str()));
    let due = stopped_at(&s).is_some_and(|t| pick_at(t, &zone) <= now);
    let mut kit_state = s.kit_state.clone();
    let mut ctx = KitCtx::new(&s.spec, &s.players, now);
    match wall::pick_step(&s.kit_state, due, running_here(state, code)) {
        wall::PickStep::Wait => Ok(false),
        wall::PickStep::Skip => {
            wall::pick_skip(&mut kit_state);
            super::save(state, code, &kit_state, ctx, now).await?;
            Ok(false)
        }
        wall::PickStep::ProposeAll(ids) => {
            wall::pick_propose_all(&mut ctx, &mut kit_state, &ids);
            super::save(state, code, &kit_state, ctx, now).await?;
            drop(guard);
            say_ready(state, code, ids.len(), true).await;
            Ok(true)
        }
        wall::PickStep::Ask(approved) => {
            let origin = origin_for(&s, sched.as_ref());
            let bot = match &origin {
                Some(o) => state.db.get_kwant(&o.kwant_id).await.ok().flatten(),
                None => None,
            };
            let (Some(mut origin), Some(bot)) = (origin, bot) else {
                // No bot to ask (deleted, or the wall never had one): the host stars by hand
                wall::pick_fail(&mut ctx, &mut kit_state);
                super::save(state, code, &kit_state, ctx, now).await?;
                return Ok(false);
            };
            // A chat that's gone since: the bot's direct chat instead
            if schedule::check_origin(state, &origin).await.is_err() {
                origin = Origin { kwant_id: bot.id.clone(), channel_id: None, thread_id: None };
            }
            if !wall::pick_start(&mut kit_state) {
                // Its tries are used up (a restart cut off the second one too): never again, and
                // the host hears it didn't work (QA F4 at 3032d327: it used to end silently)
                wall::pick_fail(&mut ctx, &mut kit_state);
                super::save(state, code, &kit_state, ctx, now).await?;
                drop(guard);
                say_failed(state, code).await;
                return Ok(false);
            }
            // Saved (claimed) before the turn starts: another tick or a restart sees `picking`
            super::save(state, code, &kit_state, ctx, now).await?;
            register(state, code, &bot.id);
            drop(guard);
            let prompt = prompt(code, &s.row.title, approved.len());
            let (st, code) = (state.clone(), code.to_string());
            tokio::spawn(async move { run_pick(st, code, origin, bot, prompt).await });
            Ok(true)
        }
    }
}

/// The pick's one prompt. No guest names and no captions: ids and previews only.
pub fn prompt(code: &str, title: &str, approved: usize) -> String {
    let shown = approved.min(SHOW_MAX);
    let pages = shown.div_ceil(PAGE);
    let sample = if approved > SHOW_MAX { format!(" (an even spread of the {approved})") } else { String::new() };
    let best = wall::BEST_MAX;
    format!(
        "Pick the best {best} photos from the photo wall {title:?} (pocket {code}), which closed with {approved} approved photos. \
         Look at {shown} of them{sample} with {TOOL_PHOTOS} (code \"{code}\", pages 1 to {pages}, {PAGE} a page), \
         then call {TOOL_PROPOSE} with code \"{code}\" and the ids of the {best} best: sharp, smiling, and not too alike. \
         Use only ids you saw. If it turns your list down, fix the list and call it again. Don't describe any photo or anyone in them, and don't comment on how people look. \
         Nothing goes to guests: the host looks your pick over first. When {TOOL_PROPOSE} says it worked, reply only \"Done.\""
    )
}

async fn run_pick(state: AppState, code: String, origin: Origin, bot: Kwant, prompt: String) {
    let in_thread = origin.thread_id.is_some();
    let message = NewMessage {
        channel_id: origin.channel_id.clone(),
        direct_kwant_id: if origin.channel_id.is_none() && !in_thread { Some(bot.id.clone()) } else { None },
        sender_role: "user".into(),
        sender_id: LABEL.into(),
        text: prompt.clone(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: prompt.len() as i64 / 4,
    };
    let extra = MessageExtra { thread_id: origin.thread_id.clone(), meta: Some(json!({ "kind": "pocket", "reason": "best_pick", "code": code })) };
    match state.db.insert_message_with(&message, &extra).await {
        Ok(_) => {
            if let Some(t) = &origin.thread_id {
                let _ = state.db.touch_thread(t).await;
            }
            if let Err(e) = crate::server::routes::run_pocket_pick_turn(&state, &bot, origin.channel_id.clone(), origin.thread_id.clone(), prompt, LABEL).await {
                tracing::warn!(code = %code, "pocket apps: the wall's best-30 turn failed: {e}");
            }
        }
        Err(e) => tracing::warn!(code = %code, "pocket apps: couldn't post the best-30 prompt: {e}"),
    }
    finish(&state, &code).await;
}

/// After the turn: still `picking` means no usable proposal came, so the pick failed (no retry).
async fn finish(state: &AppState, code: &str) {
    let lock = state.pocket.lock_for(code);
    let guard = lock.lock().await;
    let now = Utc::now();
    let mut said = None;
    match super::snapshot(state, code).await {
        Ok(Some(s)) if wall::is_picking(&s.kit_state) => {
            let mut kit_state = s.kit_state.clone();
            let mut ctx = KitCtx::new(&s.spec, &s.players, now);
            wall::pick_fail(&mut ctx, &mut kit_state);
            if let Err(e) = super::save(state, code, &kit_state, ctx, now).await {
                tracing::warn!(code = %code, "pocket apps: couldn't note the failed best-30 pick: {}", e.code);
            }
            said = Some(None);
        }
        Ok(Some(s)) => {
            let n = wall::proposed_count(&s.kit_state);
            if n > 0 {
                said = Some(Some(n));
            }
        }
        _ => {}
    }
    unregister(state, code);
    drop(guard);
    match said {
        Some(Some(n)) => say_ready(state, code, n, false).await,
        Some(None) => say_failed(state, code).await,
        None => {}
    }
}

// ---- Lines ----

fn local_date(zone: &Zone, at: DateTime<Utc>) -> String {
    zone.local(at).format("%b %-d").to_string()
}

/// "noon" and "midnight" for those, else [`schedule::clock`].
fn clock_word(t: chrono::NaiveDateTime) -> String {
    match (chrono::Timelike::hour(&t), chrono::Timelike::minute(&t)) {
        (12, 0) => "noon".into(),
        (0, 0) => "midnight".into(),
        _ => schedule::clock(t),
    }
}

/// " All the photos leave this box on Dec 28." (with its leading space), or nothing without an
/// `expires_at`.
fn leaves_line(leaves: Option<&str>) -> String {
    leaves.map(|d| format!(" All the photos leave this box on {d}.")).unwrap_or_default()
}

/// Before the host said yes: the proposal is ready to look over. `all`: 30 or fewer, so all
/// of them (no bot call). `leaves`: the pocket's `expires_at` as a local date.
pub fn ready_copy(title: &str, n: usize, all: bool, leaves: Option<&str>) -> (String, String) {
    let t = schedule::short_title(title);
    let line = if all {
        format!("{t} closed with {n} photos, so all {n} are in the best. Look them over on the card; guests see them only after you say yes.")
    } else {
        format!("I picked the best {n} photos from {t}. Look them over on the card and swap any you like; guests see them only after you say yes.")
    };
    (format!("Best photos ready: {t}"), line + &leaves_line(leaves))
}

/// The pick didn't work: the host stars by hand.
pub fn failed_copy(title: &str, leaves: Option<&str>) -> (String, String) {
    let t = schedule::short_title(title);
    let best = wall::BEST_MAX;
    let line = format!("I couldn't pick the best {best} from {t}, so guests don't see a best set yet. You can star the best ones yourself on the card.");
    (format!("Best photos not picked: {t}"), line + &leaves_line(leaves))
}

/// After the host showed the bot's proposal unchanged (design.md state 8). Only the bot's own pick
/// gets here (more than 30 approved), so there's no "all of them" line: `best` can still equal
/// `approved` if the host hid the photos the bot left out, and the line reads right then too.
pub fn shown_copy(approved: usize, best: usize, closed: Option<chrono::NaiveDateTime>, leaves: Option<String>, morning: bool) -> String {
    let mut line = String::new();
    if morning {
        line.push_str("Good morning! ");
    }
    if let Some(c) = closed {
        line.push_str(&format!("The wall closed at {} with {approved} photos. ", clock_word(c)));
    }
    line.push_str(&format!("I picked the best {best} and showed them to guests after you said yes."));
    line + &leaves_line(leaves.as_deref())
}

async fn zone_and_origin(state: &AppState, s: &Snapshot) -> (Zone, Option<Origin>) {
    let sched = state.db.pocket_schedule(&s.row.code).await.ok().flatten();
    let zone = Zone::parse(sched.as_ref().map_or("UTC", |r| r.zone.as_str()));
    (zone, origin_for(s, sched.as_ref()))
}

/// When the pocket's photos leave this box: its `expires_at` once it has ended, else (uploads
/// stopped at `closes_at` but nobody ended the wall yet, so the pocket has no `expires_at`) the
/// stop plus [`wall::KEEP_DAYS`], the rule the guests' keep line uses. The pick only runs after
/// the stop, so its lines always have a date (CI at d86a9d2d: they came out without one).
fn leaves_at(expires_at: Option<DateTime<Utc>>, stopped: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    expires_at.or_else(|| stopped.map(|t| t + chrono::Duration::days(wall::KEEP_DAYS)))
}

/// The day the pocket's photos leave this box, on the person's clock (as in the state-8 line).
fn leaves_on(zone: &Zone, s: &Snapshot) -> Option<String> {
    leaves_at(s.row.expires_at, stopped_at(s)).map(|t| local_date(zone, t))
}

async fn say_ready(state: &AppState, code: &str, n: usize, all: bool) {
    let Ok(Some(s)) = super::snapshot(state, code).await else { return };
    let (zone, origin) = zone_and_origin(state, &s).await;
    let (title, line) = ready_copy(&s.row.title, n, all, leaves_on(&zone, &s).as_deref());
    if let Some(o) = &origin {
        schedule::post(state, o, json!({ "kind": "pocket", "reason": "best_ready", "code": code, "block": null }), line.clone()).await;
    }
    schedule::notify(state, origin.as_ref(), &s, title, line).await;
}

async fn say_failed(state: &AppState, code: &str) {
    let Ok(Some(s)) = super::snapshot(state, code).await else { return };
    let (zone, origin) = zone_and_origin(state, &s).await;
    let (title, line) = failed_copy(&s.row.title, leaves_on(&zone, &s).as_deref());
    if let Some(o) = &origin {
        schedule::post(state, o, json!({ "kind": "pocket", "reason": "best_failed", "code": code, "block": null }), line.clone()).await;
    }
    schedule::notify(state, origin.as_ref(), &s, title, line).await;
}

/// The first time the host shows the proposal unchanged (from [`super::act`], under the pocket's
/// lock, after the save): the state-8 line in the pocket's chat.
pub async fn say_shown(state: &AppState, code: &str) {
    let Ok(Some(s)) = super::snapshot(state, code).await else { return };
    let (zone, origin) = zone_and_origin(state, &s).await;
    let Some(o) = origin else { return };
    let now = Utc::now();
    let (approved, best) = wall::best_counts(&s.kit_state);
    let closed = stopped_at(&s).map(|t| zone.local(t));
    let leaves = leaves_on(&zone, &s);
    let morning = chrono::Timelike::hour(&zone.local(now)) < 12;
    let line = shown_copy(approved, best, closed, leaves, morning);
    schedule::post(state, &o, json!({ "kind": "pocket", "reason": "best_shown", "code": code, "block": null }), line).await;
}

// ---- The bot's two tools ----

pub fn handles(name: &str) -> bool {
    name == TOOL_PHOTOS || name == TOOL_PROPOSE
}

pub fn tools() -> Vec<Value> {
    let code = json!({"type": "string", "description": "The wall's code, from the pick's prompt"});
    vec![
        json!({
            "name": TOOL_PHOTOS,
            "description": format!("Small previews of the approved photos of the wall whose best {} you're picking, {PAGE} a page, each after its id. \
                Only while that pick runs.", wall::BEST_MAX),
            "inputSchema": {"type": "object", "properties": {"code": code, "page": {"type": "integer", "minimum": 1}}, "required": ["code"]},
        }),
        json!({
            "name": TOOL_PROPOSE,
            "description": format!("Propose the wall's best photos: at most {} ids of its approved photos, each once. The host looks them over; \
                guests see nothing until the host shows them. Only while the pick runs.", wall::BEST_MAX),
            "inputSchema": {"type": "object", "properties": {"code": code, "ids": {"type": "array", "items": {"type": "string"}, "maxItems": wall::BEST_MAX}}, "required": ["code", "ids"]},
        }),
    ]
}

fn text(t: impl Into<String>, is_error: bool) -> (Value, bool) {
    (json!([{ "type": "text", "text": t.into() }]), is_error)
}

/// One of the two tools for `kwant_id`'s turn: content blocks and whether it's an error.
pub async fn call(state: &AppState, kwant_id: &str, name: &str, args: &Value) -> (Value, bool) {
    let code = args["code"].as_str().map(str::trim).unwrap_or_default();
    if !running_for(state, code, kwant_id) {
        return text("You aren't picking that wall's best photos now.", true);
    }
    match name {
        TOOL_PHOTOS => photos(state, code, args).await,
        TOOL_PROPOSE => propose(state, code, args).await,
        _ => text(format!("unknown tool {name}"), true),
    }
}

/// An evenly spaced sample of at most [`SHOW_MAX`] items, in order.
pub fn sample<T: Clone>(all: &[T]) -> Vec<T> {
    let n = all.len();
    if n <= SHOW_MAX {
        return all.to_vec();
    }
    (0..SHOW_MAX).map(|i| all[i * n / SHOW_MAX].clone()).collect()
}

async fn photos(state: &AppState, code: &str, args: &Value) -> (Value, bool) {
    let Ok(Some(s)) = super::snapshot(state, code).await else { return text("That wall is gone.", true) };
    if !wall::is_picking(&s.kit_state) {
        return text("That wall isn't picking its best photos now.", true);
    }
    let all = wall::approved_photos(&s.kit_state);
    let shown = sample(&all);
    let pages = shown.len().div_ceil(PAGE).max(1);
    let page = args["page"].as_u64().unwrap_or(1).max(1) as usize;
    if page > pages {
        return text(format!("There are {pages} pages."), true);
    }
    let Some(dir) = wall::wall_dir(state, code) else { return text("That wall is gone.", true) };
    let from = (page - 1) * PAGE;
    let part: Vec<wall::Photo> = shown.iter().skip(from).take(PAGE).cloned().collect();
    let mut blocks = vec![json!({ "type": "text", "text": format!(
        "Page {page} of {pages}: photos {} to {} of the {} you can look at ({} approved). Each preview follows its id.",
        from + 1, from + part.len(), shown.len(), all.len()) })];
    for p in part {
        let path = p.file_name().map(|f| dir.join(f));
        let made = match path {
            Some(path) => {
                let _slot = PREVIEWS.acquire().await;
                tokio::task::spawn_blocking(move || std::fs::read(&path).ok().and_then(|b| preview(&b, p.rotate))).await.ok().flatten()
            }
            None => None,
        };
        match made {
            Some(jpeg) => {
                blocks.push(json!({ "type": "text", "text": format!("id: {}", p.id) }));
                blocks.push(json!({ "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(jpeg), "mimeType": "image/jpeg" }));
            }
            None => blocks.push(json!({ "type": "text", "text": format!("id: {} (no preview; leave it out)", p.id) })),
        }
    }
    (Value::Array(blocks), false)
}

/// A small JPEG of `bytes` turned upright: decoded in memory with limits, never written to disk
/// (the wall's folder keeps only the photos themselves).
pub fn preview(bytes: &[u8], rotate: u16) -> Option<Vec<u8>> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(DECODE_MAX_EDGE);
    limits.max_image_height = Some(DECODE_MAX_EDGE);
    limits.max_alloc = Some(DECODE_MAX_ALLOC);
    reader.limits(limits);
    let small = reader.decode().ok()?.thumbnail(THUMB_EDGE, THUMB_EDGE);
    let upright = match rotate {
        90 => small.rotate90(),
        180 => small.rotate180(),
        270 => small.rotate270(),
        _ => small,
    };
    let mut out = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, THUMB_QUALITY);
    upright.to_rgb8().write_with_encoder(encoder).ok()?;
    Some(out)
}

async fn propose(state: &AppState, code: &str, args: &Value) -> (Value, bool) {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let Ok(Some(s)) = super::snapshot(state, code).await else { return text("That wall is gone.", true) };
    let now = Utc::now();
    let mut kit_state = s.kit_state.clone();
    let mut ctx = KitCtx::new(&s.spec, &s.players, now);
    let n = match wall::pick_propose(&mut ctx, &mut kit_state, &args["ids"]) {
        Ok(n) => n,
        Err(super::kits::KitError::BadInput(m) | super::kits::KitError::NotNow(m) | super::kits::KitError::Coded { message: m, .. }) => return text(m, true),
    };
    if super::save(state, code, &kit_state, ctx, now).await.is_err() {
        return text("Couldn't save the pick. Try once more.", true);
    }
    text(format!("Saved: {n} photos for the host to look over. Reply \"Done.\""), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn the_leave_date_is_expires_at_else_the_stop_plus_the_keep_days() {
        let closed = utc(2026, 11, 28, 12, 0);
        let ended = utc(2026, 11, 29, 8, 0);
        // Ended: the pocket's own expires_at wins
        assert_eq!(leaves_at(Some(ended + chrono::Duration::days(30)), Some(closed)), Some(utc(2026, 12, 29, 8, 0)));
        // Closed by closes_at, not ended yet: the stop plus 30 days, the guests' keep line
        assert_eq!(leaves_at(None, Some(closed)), Some(utc(2026, 12, 28, 12, 0)));
        assert_eq!(local_date(&Zone::parse("UTC"), leaves_at(None, Some(closed)).unwrap()), "Dec 28");
        // Neither: no date, and the lines drop the sentence
        assert_eq!(leaves_at(None, None), None);
    }

    #[test]
    fn the_pick_is_the_first_nine_oclock_after_uploads_stopped() {
        let utc0 = Zone::parse("UTC");
        // Closed at noon: 09:00 the next day
        assert_eq!(pick_at(utc(2026, 11, 28, 12, 0), &utc0), utc(2026, 11, 29, 9, 0));
        // Closed at 2 AM: 09:00 the same day
        assert_eq!(pick_at(utc(2026, 11, 29, 2, 0), &utc0), utc(2026, 11, 29, 9, 0));
        // Closed at exactly 09:00: the next day
        assert_eq!(pick_at(utc(2026, 11, 29, 9, 0), &utc0), utc(2026, 11, 30, 9, 0));
        // On the person's clock: 09:00 in +02:00 is 07:00 UTC
        let plus2 = Zone::parse("+02:00");
        assert_eq!(pick_at(utc(2026, 11, 28, 22, 0), &plus2), utc(2026, 11, 29, 7, 0));
    }

    #[test]
    fn a_big_wall_shows_an_even_spread_of_ninety() {
        let all: Vec<usize> = (0..212).collect();
        let s = sample(&all);
        assert_eq!(s.len(), SHOW_MAX);
        assert_eq!(s[0], 0);
        assert!(s.windows(2).all(|w| w[0] < w[1]));
        assert!(*s.last().unwrap() >= 200);
        assert_eq!(sample(&all[..40]).len(), 40);
    }

    #[test]
    fn the_prompt_has_no_names_and_asks_for_the_three_things() {
        let p = prompt("abcd2345", "Mia & Jonas's wedding", 212);
        assert!(p.contains("sharp, smiling, and not too alike"));
        assert!(p.contains(TOOL_PHOTOS) && p.contains(TOOL_PROPOSE));
        assert!(p.contains("pages 1 to 9"));
        assert!(p.contains("Don't describe any photo"));
        // copy check: no "once", so a refused list gets fixed in the same turn
        assert!(p.contains(&format!("then call {TOOL_PROPOSE} with code")), "{p}");
        assert!(p.contains("Use only ids you saw. If it turns your list down, fix the list and call it again."), "{p}");
        assert!(!p.contains(" once "), "{p}");
    }

    #[test]
    fn the_lines_follow_the_design() {
        let noon = chrono::NaiveDate::from_ymd_opt(2026, 11, 29).unwrap().and_hms_opt(12, 0, 0).unwrap();
        assert_eq!(
            shown_copy(212, 30, Some(noon), None, true),
            "Good morning! The wall closed at noon with 212 photos. I picked the best 30 and showed them to guests after you said yes."
        );
        assert_eq!(
            shown_copy(212, 30, Some(noon), Some("Dec 28".into()), false),
            "The wall closed at noon with 212 photos. I picked the best 30 and showed them to guests after you said yes. All the photos leave this box on Dec 28."
        );
        // Mira's copy check at 3032d327: lines 2, 3 and 5
        let w = "Mia & Jonas's wedding";
        let (title, ready) = ready_copy(w, 30, false, Some("Dec 28"));
        assert_eq!(title, "Best photos ready: Mia & Jonas's wedding");
        assert_eq!(
            ready,
            "I picked the best 30 photos from Mia & Jonas's wedding. Look them over on the card and swap any you like; guests see them only after you say yes. All the photos leave this box on Dec 28."
        );
        let (_, all) = ready_copy(w, 12, true, Some("Dec 28"));
        assert_eq!(
            all,
            "Mia & Jonas's wedding closed with 12 photos, so all 12 are in the best. Look them over on the card; guests see them only after you say yes. All the photos leave this box on Dec 28."
        );
        let (title, failed) = failed_copy(w, Some("Dec 28"));
        assert_eq!(title, "Best photos not picked: Mia & Jonas's wedding");
        assert_eq!(
            failed,
            "I couldn't pick the best 30 from Mia & Jonas's wedding, so guests don't see a best set yet. You can star the best ones yourself on the card. All the photos leave this box on Dec 28."
        );
        // No expires_at: the leave sentence is dropped
        for line in [ready_copy(w, 30, false, None).1, ready_copy(w, 12, true, None).1, failed_copy(w, None).1] {
            assert!(!line.contains("leave this box") && line.ends_with('.'), "{line}");
        }
        assert!(ready_copy(w, 12, true, None).1.ends_with("only after you say yes."));
    }

    #[test]
    fn previews_are_small_upright_jpegs() {
        let mut img = image::RgbImage::new(1200, 800);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = image::Rgb([(x % 256) as u8, 90, 160]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let out = preview(&png, 90).expect("a preview");
        assert_eq!(&out[..2], &[0xFF, 0xD8]);
        let back = image::load_from_memory(&out).unwrap();
        // Turned a quarter: portrait now, longest side 512
        assert_eq!(back.height(), 512);
        assert!((340..=342).contains(&back.width()), "{}", back.width());
        assert!(preview(b"not a photo", 0).is_none());
    }
}
