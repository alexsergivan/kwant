//! The Wall kit (pa14): a party photo wall. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa14/server-brief.md, API in
//! server-impl.md next to it ("Pocket API v1: wall").
//!
//! Guests scan the QR, join with a name and PIN, and send full-size photos from the browser, one
//! per request (`POST /api/pocket/<code>/photo`, raw body). Every photo is stripped of location,
//! camera and text metadata on arrival, before a byte is written ([`super::photo`]), and waits
//! for the host: nobody else sees it until the host approves it (plan section 8 rules 5 and 12).
//! The big screen shows approved photos only. The host downloads them as a zip from the person's
//! main port (`GET /api/pockets/<code>/wall.zip`).
//!
//! Photo bytes never go in the kit state or the database: they are files under
//! `<data_dir>/pocket/<code>/wall/<id>.<ext>` (folder 0700, files 0600 on Mac and Linux), and the
//! state holds one record per photo. A hidden photo's file is deleted at once (after the save
//! that hides it); its record keeps only the id and `hidden`. Everything goes 30 days after the
//! pocket ends, with the pocket (rule 11: the photos are the host's, who keeps the zip).
//!
//! Spec (`POST /api/pockets` with `kit: "wall"`): `title` (required), `when` / `event_at` /
//! `closes_at` (dates; uploads stop at `closes_at`), `host_name`, `photo_cap_mb` (1-25, default
//! 15), `pocket_cap_mb` (1-4096, default 1024), `per_guest_max` (1-500, default 40), `captions`
//! (default true). A caption comes with the photo in the `X-Caption` header (percent-encoded
//! UTF-8) and can be changed with the guest `caption` action until the host approves the photo;
//! it is cut to 80 characters and goes through the word filter (a caught one is dropped and the
//! photo says `caption_removed`).

use super::kits::{spec_date, Kit, KitCtx, KitError, KitRules, Retention, Viewer, DEFAULT_MAX_PLAYERS};
use super::photo::{self, Kind, Sniff};
use super::{keys, words, PResult, PocketError, Snapshot};
use crate::server::AppState;
use axum::body::{Body, Bytes};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const ID: &str = "wall";
const MB: u64 = 1024 * 1024;
pub const PHOTO_CAP_MB: u64 = 15;
pub const PHOTO_CAP_MB_MAX: u64 = 25;
pub const POCKET_CAP_MB: u64 = 1024;
pub const POCKET_CAP_MB_MAX: u64 = 4096;
pub const PER_GUEST_MAX: u64 = 40;
pub const PER_GUEST_MAX_MAX: u64 = 500;
pub const CAPTION_MAX: usize = 80;
const HOST_NAME_MAX: usize = 40;
/// Photos the host (or pa14b's pick) may mark as the best.
pub const BEST_MAX: usize = 30;
/// Days the photos stay after the pocket ends.
pub const KEEP_DAYS: i64 = 30;
/// Uploads stop while the disk holding the data folder would drop under this.
pub const DISK_FLOOR: u64 = 2 * 1024 * MB;
/// The upload route's own body limit: the hard ceiling of `photo_cap_mb`.
pub const UPLOAD_LIMIT: usize = (PHOTO_CAP_MB_MAX * MB) as usize;
/// Ids one host action may name.
const IDS_MAX: usize = 1000;
/// Photo uploads in flight at once across every wall (pa14 QA F2): each holds up to
/// [`UPLOAD_LIMIT`] in memory plus its stripped copy, so 4 keep it near 200 MB on a small box.
/// Others wait for a slot (their bodies stay unread meanwhile) up to [`UPLOAD_WAIT_SECS`].
pub const UPLOADS_IN_FLIGHT: usize = 4;
pub const UPLOAD_WAIT_SECS: u64 = 20;
pub static UPLOADS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(UPLOADS_IN_FLIGHT);

const SPEC_FIELDS: &[&str] = &["title", "when", "event_at", "closes_at", "host_name", "photo_cap_mb", "pocket_cap_mb", "per_guest_max", "captions"];

pub const PENDING: &str = "pending";
pub const APPROVED: &str = "approved";
pub const HIDDEN: &str = "hidden";

// ---- Spec ----

/// The limits one spec sets, in bytes and counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub photo_bytes: u64,
    pub pocket_bytes: u64,
    pub per_guest: u64,
    pub captions: bool,
}

pub fn caps(spec: &Value) -> Caps {
    let n = |f: &str, d: u64, max: u64| spec.get(f).and_then(Value::as_u64).filter(|v| (1..=max).contains(v)).unwrap_or(d);
    Caps {
        photo_bytes: n("photo_cap_mb", PHOTO_CAP_MB, PHOTO_CAP_MB_MAX) * MB,
        pocket_bytes: n("pocket_cap_mb", POCKET_CAP_MB, POCKET_CAP_MB_MAX) * MB,
        per_guest: n("per_guest_max", PER_GUEST_MAX, PER_GUEST_MAX_MAX),
        captions: spec.get("captions").and_then(Value::as_bool).unwrap_or(true),
    }
}

fn spec_str<'a>(spec: &'a Value, field: &str) -> Option<&'a str> {
    spec.get(field).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn uploads_close_at(spec: &Value) -> Option<DateTime<Utc>> {
    spec.get("closes_at").and_then(spec_date)
}

// ---- State ----

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct St {
    #[serde(default)]
    v: u32,
    #[serde(default)]
    photos: Vec<Photo>,
    /// Photos each guest has sent, hidden ones included (`per_guest_max` counts sends).
    #[serde(default)]
    uploads: BTreeMap<String, u64>,
    #[serde(default)]
    uploads_closed: bool,
    /// True once the host showed the best photos to guests (`set_best`).
    #[serde(default)]
    best_set: bool,
    // pa14b: the best-30 pick the morning after the wall closes (see `wall_pick`)
    /// The wall's bot is picking now (the card's state 6).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    picking: bool,
    /// Times a pick started: once, plus once more if a restart cut it off ([`PICK_TRIES`]).
    #[serde(default, skip_serializing_if = "is_zero8")]
    pick_tries: u8,
    /// The pick ran, failed, or isn't needed (the host chose by hand): it never starts again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pick_done: bool,
    /// The pick's best (at most 30 approved ids), for the host to look over (state 7). Never in
    /// a guest's, the lobby's or the big screen's view.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    proposed_best: Vec<String>,
    /// The proposal came from the bot's turn ([`pick_propose`]), not from "30 or fewer, so all"
    /// ([`pick_propose_all`]): only then can the best shown be the bot's (`best_by: "snap"`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    proposed_by_bot: bool,
    /// The best shown now is the pick's proposal, unchanged (`best_by: "snap"`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    best_by_snap: bool,
    /// Bot calls this wall cost, for the budget line.
    #[serde(default, skip_serializing_if = "is_zero32")]
    bot_calls: u32,
    /// The "I picked the best 30 and showed them" line went to the person's chat.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    snap_said: bool,
}

/// One photo's record. A hidden one keeps only `id` and `status`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Photo {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ext: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
    #[serde(default, skip_serializing_if = "is_zero16")]
    pub rotate: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// The caption had a word the word filter stops, so it was dropped.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub caption_removed: bool,
    pub status: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub best: bool,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

fn is_zero16(n: &u16) -> bool {
    *n == 0
}

fn is_zero8(n: &u8) -> bool {
    *n == 0
}

fn is_zero32(n: &u32) -> bool {
    *n == 0
}

impl Photo {
    fn hidden(id: &str) -> Photo {
        Photo { id: id.to_string(), status: HIDDEN.into(), ..Photo::default() }
    }

    pub(super) fn file_name(&self) -> Option<String> {
        Kind::from_ext(&self.ext).map(|k| format!("{}.{}", self.id, k.ext()))
    }
}

fn load(state: &Value) -> St {
    serde_json::from_value(state.clone()).unwrap_or_default()
}

fn store(state: &mut Value, s: &St) {
    if let Ok(v) = serde_json::to_value(s) {
        *state = v;
    }
}

/// Bytes the photos still on disk take (hidden ones are gone).
fn used_bytes(s: &St) -> u64 {
    s.photos.iter().filter(|p| p.status != HIDDEN).map(|p| p.bytes).sum()
}

fn uploads_open(ctx: &KitCtx, s: &St) -> bool {
    !s.uploads_closed && !uploads_close_at(ctx.spec).is_some_and(|t| t <= ctx.now)
}

fn not_now(m: impl Into<String>) -> KitError {
    KitError::NotNow(m.into())
}

fn bad(m: impl Into<String>) -> KitError {
    KitError::BadInput(m.into())
}

/// `{id}` or `{ids: [...]}` from a host action's body.
fn ids(body: &Value) -> Result<Vec<String>, KitError> {
    let list: Vec<&Value> = match (body.get("id"), body.get("ids")) {
        (Some(id), None) => vec![id],
        (None, Some(Value::Array(a))) => a.iter().collect(),
        _ => return Err(bad("Send id, or ids as a list.")),
    };
    if list.len() > IDS_MAX {
        return Err(bad(format!("At most {IDS_MAX} photos at once.")));
    }
    list.into_iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| bad("Photo ids are text."))).collect()
}

/// A guest's first name for others (the host sees full names).
fn first_name(ctx: &KitCtx, player_id: &str) -> Option<String> {
    ctx.player(player_id).and_then(|p| p.name.split_whitespace().next()).map(str::to_string)
}

fn full_name(ctx: &KitCtx, player_id: &str) -> Option<String> {
    ctx.player(player_id).map(|p| p.name.clone())
}

pub struct Wall;

/// A guest's creature on the TV and the wall: one of the kit's 8, from the guest id (FNV-1a), so
/// joining stays one name field.
pub fn creature(player_id: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in player_id.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h % 8
}

/// `closes_at + 30 days` as a date: when the photos leave, for the keep line.
fn delete_on(spec: &Value) -> Option<String> {
    uploads_close_at(spec).map(|t| (t + chrono::Duration::days(KEEP_DAYS)).date_naive().to_string())
}

impl Wall {
    fn facts(ctx: &KitCtx, s: &St) -> Value {
        let mut v = json!({
            "title": spec_str(ctx.spec, "title"),
            "host_name": spec_str(ctx.spec, "host_name"),
            "when": ctx.spec.get("when").or_else(|| ctx.spec.get("event_at")).and_then(spec_date),
            "closes_at": uploads_close_at(ctx.spec),
            "delete_on": delete_on(ctx.spec),
            "state": if uploads_open(ctx, s) { "open" } else { "closed" },
            "captions": caps(ctx.spec).captions,
            "best_set": s.best_set,
        });
        // pa14b: guests may see that the bot picked the best shown (never what it proposed)
        if s.best_set && s.best_by_snap {
            v["best_by"] = json!("snap");
        }
        v
    }

    /// An approved photo as a guest or the big screen sees it: first name, no player id.
    fn wall_photo(ctx: &KitCtx, p: &Photo, s: &St) -> Value {
        json!({
            "id": p.id,
            "by_name": first_name(ctx, &p.by),
            "creature": creature(&p.by),
            "caption": p.caption,
            "at": p.at,
            "w": p.w,
            "h": p.h,
            "rotate": p.rotate,
            "best": p.best && s.best_set,
        })
    }

    /// One of the guest's own photos (pending or approved; hidden ones are dropped).
    fn own_photo(p: &Photo) -> Value {
        json!({
            "id": p.id,
            "status": p.status,
            "caption": p.caption,
            "caption_removed": p.caption_removed,
            "at": p.at,
            "w": p.w,
            "h": p.h,
            "rotate": p.rotate,
        })
    }

    fn host_photo(ctx: &KitCtx, p: &Photo) -> Value {
        json!({
            "id": p.id,
            "by": p.by,
            "by_name": full_name(ctx, &p.by),
            "creature": creature(&p.by),
            "caption": p.caption,
            "caption_removed": p.caption_removed,
            "at": p.at,
            "bytes": p.bytes,
            "type": Kind::from_ext(&p.ext).map(Kind::mime),
            "w": p.w,
            "h": p.h,
            "rotate": p.rotate,
            "status": p.status,
            "best": p.best,
        })
    }

    fn counts(ctx: &KitCtx, s: &St) -> Value {
        let n = |st: &str| s.photos.iter().filter(|p| p.status == st).count();
        json!({
            "total": s.photos.len(),
            "waiting": n(PENDING),
            "approved": n(APPROVED),
            "hidden": n(HIDDEN),
            "guests": ctx.players.len(),
            "senders": s.uploads.len(),
        })
    }

    /// The host's numbers: counts, space used and the cap, best, when the photos leave.
    fn host_numbers(ctx: &KitCtx, s: &St) -> Value {
        let mut v = Self::facts(ctx, s);
        v["counts"] = Self::counts(ctx, s);
        v["used_mb"] = json!((used_bytes(s) as f64 / MB as f64 * 10.0).round() / 10.0);
        v["pocket_cap_mb"] = json!(caps(ctx.spec).pocket_bytes / MB);
        v["best"] = json!(s.photos.iter().filter(|p| p.best).map(|p| p.id.as_str()).collect::<Vec<_>>());
        v["best_max"] = json!(BEST_MAX);
        v["keep_days"] = json!(KEEP_DAYS);
        // pa14b: "snap" while the best shown is the pick's proposal unchanged (the Picked
        // sticker), "host" for any swap or hand pick, null with no best
        v["best_by"] = match (s.photos.iter().any(|p| p.best), s.best_by_snap) {
            (false, _) => Value::Null,
            (true, true) => json!("snap"),
            (true, false) => json!("host"),
        };
        v["picking"] = json!(s.picking);
        v["proposed_best"] = json!(s.proposed_best);
        v["budget"] = budget(s);
        v
    }

    /// The spec's limits as anyone with the link may see them: no counts, so no seat needed.
    fn public_limits(ctx: &KitCtx) -> Value {
        let c = caps(ctx.spec);
        json!({
            "photo_cap_mb": c.photo_bytes / MB,
            "per_guest_max": c.per_guest,
            "caption_max": CAPTION_MAX,
            "types": ["image/jpeg", "image/png", "image/webp"],
        })
    }

    fn guest_view(ctx: &KitCtx, s: &St, me: &str) -> Value {
        let sent = s.uploads.get(me).copied().unwrap_or(0);
        let mut v = Self::facts(ctx, s);
        v["limits"] = Self::public_limits(ctx);
        v["limits"]["mine_count"] = json!(sent);
        // Newest first; the guest page shows 12 at a time
        v["wall"] = json!(s.photos.iter().rev().filter(|p| p.status == APPROVED).map(|p| Self::wall_photo(ctx, p, s)).collect::<Vec<_>>());
        v["mine"] = json!(s.photos.iter().rev().filter(|p| p.by == me && p.status != HIDDEN).map(Self::own_photo).collect::<Vec<_>>());
        v
    }

    fn host_view(ctx: &KitCtx, s: &St) -> Value {
        let mut v = Self::host_numbers(ctx, s);
        let c = caps(ctx.spec);
        v["limits"] = json!({ "photo_cap_mb": c.photo_bytes / MB, "pocket_cap_mb": c.pocket_bytes / MB, "per_guest_max": c.per_guest, "caption_max": CAPTION_MAX });
        // The queue oldest first (the card groups it by `by`); the wall newest first
        v["queue"] = json!(s.photos.iter().filter(|p| p.status == PENDING).map(|p| Self::host_photo(ctx, p)).collect::<Vec<_>>());
        v["wall"] = json!(s.photos.iter().rev().filter(|p| p.status == APPROVED).map(|p| Self::host_photo(ctx, p)).collect::<Vec<_>>());
        v
    }

    fn screen_view(ctx: &KitCtx, s: &St) -> Value {
        // Oldest first, so the slideshow can loop
        let photos: Vec<Value> = s
            .photos
            .iter()
            .filter(|p| p.status == APPROVED)
            .map(|p| {
                let mut w = Self::wall_photo(ctx, p, s);
                if let Some(o) = w.as_object_mut() {
                    o.remove("at");
                }
                w
            })
            .collect();
        json!({
            "title": spec_str(ctx.spec, "title"),
            "host_name": spec_str(ctx.spec, "host_name"),
            "state": if uploads_open(ctx, s) { "open" } else { "closed" },
            "best_set": s.best_set,
            "count": photos.len(),
            "photos": photos,
        })
    }

    fn approve(ctx: &mut KitCtx, s: &mut St, id: &str) -> Result<(), KitError> {
        let p = s.photos.iter_mut().find(|p| p.id == id).ok_or_else(|| bad(format!("There's no photo {id}.")))?;
        match p.status.as_str() {
            HIDDEN => Err(not_now("That photo was hidden, so it's gone.")),
            APPROVED => Ok(()),
            _ => {
                p.status = APPROVED.into();
                ctx.event(None, "photo_approved", json!({ "id": id }));
                Ok(())
            }
        }
    }

    fn hide(ctx: &mut KitCtx, s: &mut St, id: &str) -> Result<(), KitError> {
        let p = s.photos.iter_mut().find(|p| p.id == id).ok_or_else(|| bad(format!("There's no photo {id}.")))?;
        if p.status == HIDDEN {
            return Ok(());
        }
        *p = Photo::hidden(id);
        ctx.event(None, "photo_hidden", json!({ "id": id }));
        // pa14b: a hidden photo leaves the pick's proposal too
        s.proposed_best.retain(|x| x != id);
        // pa14 QA F4: once no photo is among the best any more, guests stop seeing a best
        if !s.photos.iter().any(|p| p.best) {
            s.best_set = false;
            s.best_by_snap = false;
        }
        Ok(())
    }

    /// A guest edits the caption of one of their own photos while it waits for the host.
    fn guest_caption(ctx: &mut KitCtx, s: &mut St, me: &str, body: &Value) -> Result<(), KitError> {
        let id = body.get("id").and_then(Value::as_str).ok_or_else(|| bad("Send the photo's id."))?;
        let raw = match body.get("caption") {
            None | Some(Value::Null) => "",
            Some(Value::String(t)) => t.as_str(),
            Some(_) => return Err(bad("caption must be text.")),
        };
        let p = s.photos.iter_mut().find(|p| p.id == id && p.by == me && p.status != HIDDEN).ok_or_else(|| bad("That isn't one of your photos."))?;
        if p.status != PENDING {
            return Err(not_now("The host has put that photo on the wall, so its caption stays."));
        }
        let (caption, removed) = caption_of(ctx.spec, raw);
        p.caption = caption;
        p.caption_removed = removed;
        ctx.event(Some(me), "caption_set", json!({ "id": id }));
        Ok(())
    }
}

impl Kit for Wall {
    fn id(&self) -> &'static str {
        ID
    }

    fn rules(&self, _spec: &Value) -> KitRules {
        KitRules {
            retention: Retention::AfterEnd { days: KEEP_DAYS },
            needs_pin: true,
            max_players: DEFAULT_MAX_PLAYERS,
            guest_actions: &["caption"],
            host_actions: &["approve", "hide", "approve_all_from", "set_best"],
        }
    }

    fn validate_spec(&self, spec: &Value) -> Result<(), String> {
        let obj = spec.as_object().ok_or("The spec must be a JSON object.")?;
        if let Some(k) = obj.keys().find(|k| !SPEC_FIELDS.contains(&k.as_str())) {
            return Err(format!("The Wall doesn't take {k:?}. It takes: {}.", SPEC_FIELDS.join(", ")));
        }
        let title = spec_str(spec, "title").ok_or("The spec needs a title.")?;
        if title.chars().count() > 80 {
            return Err("title is at most 80 characters.".into());
        }
        if words::blocked_text(title) {
            return Err("The title has a word the guest page won't show. Pick another.".into());
        }
        for f in ["when", "event_at", "closes_at"] {
            if let Some(v) = spec.get(f).filter(|v| !v.is_null()) {
                spec_date(v).ok_or(format!("{f} must be a date (YYYY-MM-DD) or an RFC 3339 time."))?;
            }
        }
        if let Some(v) = spec.get("host_name").filter(|v| !v.is_null()) {
            let t = v.as_str().ok_or("host_name must be text.")?;
            if t.chars().count() > HOST_NAME_MAX {
                return Err(format!("host_name is at most {HOST_NAME_MAX} characters."));
            }
            if words::blocked_text(t) {
                return Err("host_name has a word the guest page won't show.".into());
            }
        }
        let num = |f: &str, max: u64| -> Result<Option<u64>, String> {
            match spec.get(f) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v.as_u64().filter(|n| (1..=max).contains(n)).map(Some).ok_or(format!("{f} must be a whole number from 1 to {max}.")),
            }
        };
        let photo = num("photo_cap_mb", PHOTO_CAP_MB_MAX)?.unwrap_or(PHOTO_CAP_MB);
        let pocket = num("pocket_cap_mb", POCKET_CAP_MB_MAX)?.unwrap_or(POCKET_CAP_MB);
        if pocket < photo {
            return Err("pocket_cap_mb must be at least photo_cap_mb.".into());
        }
        num("per_guest_max", PER_GUEST_MAX_MAX)?;
        match spec.get("captions") {
            None | Some(Value::Null) | Some(Value::Bool(_)) => {}
            Some(_) => return Err("captions must be true or false.".into()),
        }
        Ok(())
    }

    fn init(&self, _spec: &Value) -> Value {
        let mut v = Value::Null;
        store(&mut v, &St { v: 1, ..St::default() });
        v
    }

    fn on_remove(&self, ctx: &mut KitCtx, state: &mut Value, player_id: &str) {
        // A removed guest's photos still waiting for the host go (approved ones stay the host's)
        let mut s = load(state);
        let waiting: Vec<String> = s.photos.iter().filter(|p| p.status == PENDING && p.by == player_id).map(|p| p.id.clone()).collect();
        for id in waiting {
            let _ = Self::hide(ctx, &mut s, &id);
        }
        store(state, &s);
    }

    fn act(&self, ctx: &mut KitCtx, state: &mut Value, actor: &Viewer, name: &str, body: &Value) -> Result<(), KitError> {
        let mut s = load(state);
        if let Viewer::Guest(me) = actor {
            if name != "caption" {
                return Err(bad("Only the host can do this."));
            }
            Self::guest_caption(ctx, &mut s, me, body)?;
            store(state, &s);
            return Ok(());
        }
        if *actor != Viewer::Host {
            return Err(bad("Only the host can do this."));
        }
        match name {
            "approve" => {
                for id in ids(body)? {
                    Self::approve(ctx, &mut s, &id)?;
                }
            }
            "hide" => {
                for id in ids(body)? {
                    Self::hide(ctx, &mut s, &id)?;
                }
            }
            "approve_all_from" => {
                let who = body.get("guest").or_else(|| body.get("player_id")).and_then(Value::as_str).ok_or_else(|| bad("Send guest: the guest's id."))?;
                let waiting: Vec<String> = s.photos.iter().filter(|p| p.status == PENDING && p.by == who).map(|p| p.id.clone()).collect();
                for id in waiting {
                    Self::approve(ctx, &mut s, &id)?;
                }
            }
            "set_best" => {
                let list = ids(body)?;
                let chosen: BTreeSet<String> = list.into_iter().collect();
                if chosen.len() > BEST_MAX {
                    return Err(bad(format!("At most {BEST_MAX} best photos.")));
                }
                for id in &chosen {
                    match s.photos.iter().find(|p| &p.id == id) {
                        None => return Err(bad(format!("There's no photo {id}."))),
                        Some(p) if p.status != APPROVED => return Err(not_now("Only approved photos can be among the best.")),
                        Some(_) => {}
                    }
                }
                for p in s.photos.iter_mut() {
                    p.best = chosen.contains(&p.id);
                }
                // Shown to guests from now on; an empty list takes the best away again
                s.best_set = !chosen.is_empty();
                // pa14b: the bot's proposal unchanged counts as the bot's ("snap"); any swap, a
                // hand pick, or "30 or fewer, so all" (no bot picked anything) is the host's.
                // A hand pick also means no pick ever runs after it.
                let proposal: BTreeSet<String> = s.proposed_best.iter().cloned().collect();
                s.best_by_snap = s.proposed_by_bot && !chosen.is_empty() && chosen == proposal;
                s.pick_done = true;
                s.picking = false;
                ctx.event(None, "best_set", json!({ "count": chosen.len() }));
                if s.best_by_snap && !s.snap_said {
                    s.snap_said = true;
                    ctx.event(None, SNAP_SHOWN_EVENT, json!({ "count": chosen.len() }));
                }
            }
            _ => return Err(bad(format!("unknown action {name}"))),
        }
        store(state, &s);
        Ok(())
    }

    fn tick(&self, ctx: &mut KitCtx, state: &mut Value) -> bool {
        let mut s = load(state);
        if s.uploads_closed || !uploads_close_at(ctx.spec).is_some_and(|t| t <= ctx.now) {
            return false;
        }
        s.uploads_closed = true;
        ctx.event(None, "uploads_closed", json!({}));
        store(state, &s);
        true
    }

    fn ticks_unwatched(&self) -> bool {
        true
    }

    fn host_actions_after_end(&self) -> &'static [&'static str] {
        // pa14b: the best 30 are picked the morning after; the host can still show them
        &["set_best"]
    }

    fn view(&self, ctx: &KitCtx, state: &Value, viewer: &Viewer) -> Value {
        let s = load(state);
        match viewer {
            Viewer::Guest(me) => Self::guest_view(ctx, &s, me),
            Viewer::Host => Self::host_view(ctx, &s),
            Viewer::BigScreen => Self::screen_view(ctx, &s),
        }
    }

    fn lobby(&self, ctx: &KitCtx, state: &Value) -> Value {
        let s = load(state);
        let mut v = Self::facts(ctx, &s);
        v["count"] = json!(s.photos.iter().filter(|p| p.status == APPROVED).count());
        // So the join screen can say "up to 15 MB each, 40 each" before there's a seat
        v["limits"] = Self::public_limits(ctx);
        v
    }

    /// For the card: the host view's numbers without the photo lists.
    fn host_summary(&self, ctx: &KitCtx, state: &Value) -> Value {
        Self::host_numbers(ctx, &load(state))
    }
}

// ---- pa14b: the best-30 pick ----

/// What a `set_best` emits the first time the host shows the pick's proposal unchanged: the
/// pocket core then posts the "I picked the best 30 and showed them" line ([`super::wall_pick`]).
pub const SNAP_SHOWN_EVENT: &str = "snap_best_shown";
/// Times a pick may start: once, and once more on the next tick after a restart cut it off.
pub const PICK_TRIES: u8 = 2;
/// The pick's one bot call, roughly, for the budget line (one turn with up to 90 small previews).
pub const USD_PER_PICK_CALL: f64 = 0.10;

/// What the scheduled pick does with one wall now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickStep {
    /// Nothing yet (not due, or running here), or never again (done).
    Wait,
    /// No pick: the host chose by hand, or nothing is approved. Mark it done.
    Skip,
    /// 30 or fewer approved: all of them are the proposal, no bot call.
    ProposeAll(Vec<String>),
    /// More than 30 approved: the bot picks from these.
    Ask(Vec<String>),
}

fn approved_ids(s: &St) -> Vec<String> {
    s.photos.iter().filter(|p| p.status == APPROVED).map(|p| p.id.clone()).collect()
}

/// The next step for a wall's pick. `due`: it is the morning after uploads stopped. `running_here`:
/// this process runs its pick now (state saying `picking` without that is a restart's leftover).
pub fn pick_step(state: &Value, due: bool, running_here: bool) -> PickStep {
    let s = load(state);
    if s.pick_done || (s.picking && running_here) {
        return PickStep::Wait;
    }
    // A host who already chose by hand is never overridden
    if s.best_set || s.photos.iter().any(|p| p.best) {
        return PickStep::Skip;
    }
    if !due {
        return PickStep::Wait;
    }
    let approved = approved_ids(&s);
    match approved.len() {
        0 => PickStep::Skip,
        n if n <= BEST_MAX => PickStep::ProposeAll(approved),
        _ => PickStep::Ask(approved),
    }
}

/// The pick won't run (the host chose, nothing approved, or no bot to ask): never again.
pub fn pick_skip(state: &mut Value) {
    let mut s = load(state);
    s.picking = false;
    s.pick_done = true;
    store(state, &s);
}

/// 30 or fewer approved: they're all proposed, without a bot call.
pub fn pick_propose_all(ctx: &mut KitCtx, state: &mut Value, ids: &[String]) {
    let mut s = load(state);
    s.proposed_best = ids.iter().take(BEST_MAX).cloned().collect();
    s.proposed_by_bot = false;
    s.picking = false;
    s.pick_done = true;
    ctx.event(None, "best_proposed", json!({ "count": s.proposed_best.len(), "by": "all" }));
    store(state, &s);
}

/// The bot's turn starts: `picking`, one more try and one more bot call. `false` (and done, not
/// picking) once [`PICK_TRIES`] are used up: a pick is never retried in a loop.
pub fn pick_start(state: &mut Value) -> bool {
    let mut s = load(state);
    if s.pick_tries >= PICK_TRIES {
        s.picking = false;
        s.pick_done = true;
        store(state, &s);
        return false;
    }
    s.picking = true;
    s.pick_tries += 1;
    s.bot_calls += 1;
    store(state, &s);
    true
}

/// The turn failed, the bot is gone, or it never proposed: back to not picking, no proposal,
/// and no pick again.
pub fn pick_fail(ctx: &mut KitCtx, state: &mut Value) {
    let mut s = load(state);
    s.picking = false;
    s.pick_done = true;
    s.proposed_best.clear();
    s.proposed_by_bot = false;
    ctx.event(None, "best_pick_failed", json!({}));
    store(state, &s);
}

/// The pick ran, failed or won't run: nothing more to do for it.
pub fn pick_settled(state: &Value) -> bool {
    load(state).pick_done
}

/// (approved photos, best photos) for the state-8 line.
pub fn best_counts(state: &Value) -> (usize, usize) {
    let s = load(state);
    (s.photos.iter().filter(|p| p.status == APPROVED).count(), s.photos.iter().filter(|p| p.best).count())
}

pub fn is_picking(state: &Value) -> bool {
    load(state).picking
}

pub fn proposed_count(state: &Value) -> usize {
    load(state).proposed_best.len()
}

/// The ids the bot may look at while picking: the approved photos (a placeholder record for
/// each), oldest first.
pub fn approved_photos(state: &Value) -> Vec<Photo> {
    load(state).photos.into_iter().filter(|p| p.status == APPROVED).collect()
}

/// Shortens text from the bot before it goes back to it in a refusal.
fn quoted(id: &str) -> String {
    let t: String = id.chars().filter(|c| !c.is_control()).take(40).collect();
    format!("{t:?}")
}

/// The bot's answer (`pocket_wall_propose_best`): at most 30 approved photos of this wall, no
/// repeats. A refusal leaves the pick running, so the bot can fix its list in the same turn.
pub fn pick_propose(ctx: &mut KitCtx, state: &mut Value, ids: &Value) -> Result<usize, KitError> {
    let mut s = load(state);
    if !s.picking {
        return Err(not_now("This wall isn't picking its best photos now."));
    }
    let list = ids.as_array().ok_or_else(|| bad("ids must be a list of photo ids."))?;
    if list.is_empty() {
        return Err(bad("Send at least one photo id."));
    }
    if list.len() > BEST_MAX {
        return Err(bad(format!("At most {BEST_MAX} photos; that list has {}.", list.len())));
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(list.len());
    for v in list {
        let id = v.as_str().ok_or_else(|| bad("Photo ids are text."))?;
        if !seen.insert(id) {
            return Err(bad(format!("{} is in the list twice.", quoted(id))));
        }
        if !s.photos.iter().any(|p| p.id == id && p.status == APPROVED) {
            return Err(bad(format!("{} isn't one of this wall's approved photos.", quoted(id))));
        }
        out.push(id.to_string());
    }
    let n = out.len();
    s.proposed_best = out;
    s.proposed_by_bot = true;
    s.picking = false;
    s.pick_done = true;
    ctx.event(None, "best_proposed", json!({ "count": n, "by": "snap" }));
    store(state, &s);
    Ok(n)
}

/// The host's budget line: the pick's one bot call, in rough dollars.
fn budget(s: &St) -> Value {
    let usd = (f64::from(s.bot_calls) * USD_PER_PICK_CALL * 100.0).round() / 100.0;
    let line = match s.bot_calls {
        0 => format!(
            "No bot calls so far. If more than {BEST_MAX} photos are on the wall when it closes, picking the best {BEST_MAX} the next morning takes 1 bot call, about ${USD_PER_PICK_CALL:.2}."
        ),
        1 => format!("1 bot call so far, about ${usd:.2} on your account, to pick the best {BEST_MAX}."),
        n => format!("{n} bot calls so far, about ${usd:.2} on your account, to pick the best {BEST_MAX}."),
    };
    json!({ "bot_calls": s.bot_calls, "usd": usd, "line": line })
}

// ---- Files ----

/// `<data_dir>/pocket/<code>`: `None` for a code that isn't one (never a path from a request).
fn pocket_dir(state: &AppState, code: &str) -> Option<PathBuf> {
    keys::valid_code(code).then(|| state.config.data_dir.join("pocket").join(code))
}

pub(super) fn wall_dir(state: &AppState, code: &str) -> Option<PathBuf> {
    pocket_dir(state, code).map(|d| d.join("wall"))
}

fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // `mode` above applies only to folders it made; the wall folder is always 0700
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Writes a new file (never over an old one), 0600 on Mac and Linux.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Bytes free for this user on the disk holding `path`.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is a valid NUL-terminated path for the whole call; `s` is a plain out-struct.
    unsafe {
        let mut s: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut s) != 0 {
            return None;
        }
        Some((s.f_bavail as u64).saturating_mul(s.f_frsize as u64))
    }
}

#[cfg(windows)]
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free: u64 = 0;
    // SAFETY: `wide` is NUL-terminated and lives for the call; the null pointers are optional outs.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut())
    };
    (ok != 0).then_some(free)
}

#[cfg(not(any(unix, windows)))]
fn free_bytes(_: &Path) -> Option<u64> {
    None
}

fn disk_free(state: &AppState) -> Option<u64> {
    if let Some(fake) = state.pocket.fake_free_disk() {
        return Some(fake);
    }
    free_bytes(&state.config.data_dir)
}

/// After every save of a wall pocket (under its lock): deletes files no record still points at,
/// so a hidden photo's file goes at once.
pub(super) async fn prune(state: &AppState, s: &Snapshot) {
    let Some(dir) = wall_dir(state, &s.row.code) else { return };
    let keep: BTreeSet<String> = load(&s.kit_state).photos.iter().filter(|p| p.status != HIDDEN).filter_map(Photo::file_name).collect();
    let res = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !keep.contains(&name) {
                std::fs::remove_file(e.path())?;
            }
        }
        Ok(())
    })
    .await;
    if !matches!(res, Ok(Ok(()))) {
        tracing::warn!(code = %s.row.code, "pocket apps: couldn't delete a hidden wall photo; it goes with the pocket");
    }
}

/// With the pocket: its whole folder (photos first). Missing folders are fine.
pub(super) async fn remove_files(state: &AppState, code: &str) {
    let Some(dir) = pocket_dir(state, code) else { return };
    let res = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        match std::fs::remove_dir_all(dir.join("wall")) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        match std::fs::remove_dir(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    })
    .await;
    if !matches!(res, Ok(Ok(()))) {
        tracing::warn!(code = %code, "pocket apps: couldn't delete a deleted pocket's photo folder");
    }
}

// ---- Upload ----

fn refused(status: StatusCode, code: &'static str, line: impl Into<String>) -> PocketError {
    PocketError::new(status, code, line)
}

pub fn too_big(photo_bytes: u64) -> PocketError {
    refused(StatusCode::PAYLOAD_TOO_LARGE, "too_big", format!("Too big: photos can be up to {} MB.", photo_bytes / MB)).with(json!({ "limit": photo_bytes / MB }))
}

/// A caption as stored: cut to 80 characters, links dropped, and `(None, true)` when the word
/// filter stops it (the queue says "caption removed"). A wall without captions keeps none.
pub fn caption_of(spec: &Value, raw: &str) -> (Option<String>, bool) {
    if !caps(spec).captions {
        return (None, false);
    }
    let cut: String = raw.trim().chars().take(CAPTION_MAX).collect();
    match words::clean_text(&cut, CAPTION_MAX) {
        Ok(t) => (Some(t), false),
        Err(words::TextError::NotAllowed) => (None, true),
        Err(_) => (None, false),
    }
}

/// The upload's `X-Caption` header: URI-encoded UTF-8 (a header can't carry it raw).
pub fn header_caption(spec: &Value, raw: Option<&[u8]>) -> PResult<(Option<String>, bool)> {
    let Some(raw) = raw else { return Ok((None, false)) };
    let text = percent_encoding::percent_decode(raw)
        .decode_utf8()
        .map_err(|_| refused(StatusCode::BAD_REQUEST, "bad_caption", "The caption must be URI-encoded text."))?;
    Ok(caption_of(spec, &text))
}

/// The caps of a wall pocket, or 404 for any other kit.
pub fn wall_caps(s: &Snapshot) -> PResult<Caps> {
    if s.row.kit != ID {
        return Err(PocketError::not_found());
    }
    Ok(caps(&s.spec))
}

/// One photo from guest `player_id`: checked, stripped, written, then recorded (pending).
/// Returns the new record's id.
pub async fn add_photo(state: &AppState, code: &str, player_id: &str, body: Bytes, caption: (Option<String>, bool)) -> PResult<String> {
    let kind = match photo::sniff(&body) {
        Sniff::Photo(k) => k,
        Sniff::Heic => {
            return Err(refused(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "heic",
                "This photo type can't go up. Try sharing it from Photos, which makes a JPEG.",
            ))
        }
        Sniff::Other => return Err(refused(StatusCode::UNSUPPORTED_MEDIA_TYPE, "not_photo", "Not a photo: this wall takes photos only (JPEG, PNG or WebP).")),
    };
    let stripped = tokio::task::spawn_blocking(move || photo::strip(kind, &body))
        .await
        .map_err(|_| PocketError::internal())?
        .map_err(|_| refused(StatusCode::UNSUPPORTED_MEDIA_TYPE, "broken_photo", "That photo didn't arrive whole. Try sending it again."))?;
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = super::snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    let c = wall_caps(&s)?;
    let now = Utc::now();
    super::open_check(&s, now)?;
    let mut st = load(&s.kit_state);
    let ctx = KitCtx::new(&s.spec, &s.players, now);
    if !uploads_open(&ctx, &st) {
        return Err(refused(StatusCode::GONE, "closed", "The wall has stopped taking photos."));
    }
    let sent = st.uploads.get(player_id).copied().unwrap_or(0);
    if sent >= c.per_guest {
        return Err(refused(StatusCode::CONFLICT, "guest_max", format!("You've added {} photos, the most each guest can.", c.per_guest))
            .with(json!({ "limit": c.per_guest })));
    }
    let len = stripped.bytes.len() as u64;
    if used_bytes(&st) + len > c.pocket_bytes {
        return Err(refused(StatusCode::INSUFFICIENT_STORAGE, "wall_full", "The wall is full for now. The host can make room."));
    }
    if disk_free(state).is_some_and(|free| free < DISK_FLOOR.saturating_add(len)) {
        return Err(refused(StatusCode::INSUFFICIENT_STORAGE, "disk_low", "The wall is full for now: the host's computer is almost out of space."));
    }
    let dir = wall_dir(state, code).ok_or_else(PocketError::not_found)?;
    let id = keys::new_photo_id();
    let path = dir.join(format!("{id}.{}", stripped.kind.ext()));
    let (write_path, bytes) = (path.clone(), stripped.bytes);
    tokio::task::spawn_blocking(move || {
        make_private_dir(&dir)?;
        write_private(&write_path, &bytes)
    })
    .await
    .map_err(|_| PocketError::internal())?
    .map_err(|e| {
        tracing::warn!(code = %code, "pocket apps: couldn't write a wall photo: {}", e.kind());
        PocketError::internal()
    })?;
    st.photos.push(Photo {
        id: id.clone(),
        by: player_id.to_string(),
        at: Some(now),
        bytes: len,
        ext: stripped.kind.ext().into(),
        w: stripped.w,
        h: stripped.h,
        rotate: stripped.rotate,
        caption: caption.0,
        caption_removed: caption.1,
        status: PENDING.into(),
        best: false,
    });
    *st.uploads.entry(player_id.to_string()).or_default() += 1;
    let mut kit_state = s.kit_state.clone();
    store(&mut kit_state, &st);
    let mut ctx = KitCtx::new(&s.spec, &s.players, now);
    ctx.event(Some(player_id), "photo_added", json!({ "id": id }));
    if let Err(e) = super::save(state, code, &kit_state, ctx, now).await {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e);
    }
    Ok(id)
}

// ---- Serving ----

/// One photo for `viewer`: approved ones for anyone signed in to the pocket (guest, host or
/// screen), a pending one for its sender and the host only. Anything else is the same 404, so
/// nobody learns a hidden or someone else's waiting photo exists.
pub async fn photo_response(state: &AppState, s: &Snapshot, id: &str, viewer: &Viewer, headers: &HeaderMap) -> PResult<Response> {
    wall_caps(s)?;
    let gone = || refused(StatusCode::NOT_FOUND, "no_such_photo", "That photo isn't here.");
    if !keys::valid_photo_id(id) {
        return Err(gone());
    }
    let st = load(&s.kit_state);
    let p = st.photos.iter().find(|p| p.id == id && p.status != HIDDEN).ok_or_else(gone)?;
    let pending = p.status == PENDING;
    let may = match viewer {
        Viewer::Host => true,
        Viewer::Guest(me) => !pending || p.by == *me,
        Viewer::BigScreen => !pending,
    };
    if !may {
        return Err(gone());
    }
    let kind = Kind::from_ext(&p.ext).ok_or_else(gone)?;
    let etag = format!("\"{}\"", p.id);
    // Approved: the browser keeps it but asks again each time (a hidden photo then 404s);
    // pending: never kept
    let cache = if pending { "private, no-store" } else { "private, no-cache" };
    let mut h = vec![
        (header::CONTENT_TYPE, HeaderValue::from_static(kind.mime())),
        (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
        (header::CONTENT_DISPOSITION, HeaderValue::from_static("inline")),
    ];
    if !pending {
        h.push((header::ETAG, HeaderValue::from_str(&etag).map_err(|_| PocketError::internal())?));
        if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).is_some_and(|v| v.split(',').any(|t| t.trim() == etag)) {
            let mut res = StatusCode::NOT_MODIFIED.into_response();
            res.headers_mut().extend(h.into_iter().filter(|(k, _)| k != header::CONTENT_TYPE));
            return Ok(res);
        }
    }
    let path = wall_dir(state, &s.row.code).ok_or_else(gone)?.join(format!("{}.{}", p.id, kind.ext()));
    let bytes = tokio::fs::read(&path).await.map_err(|_| gone())?;
    let mut res = Body::from(bytes).into_response();
    res.headers_mut().extend(h);
    Ok(res)
}

// ---- Zip (main port, the person) ----

/// A zip entry's name: `001-mia-2114.jpg` (number, the guest's first name, upload time HHMM in
/// the local time of the computer kwant runs on, the host's own clock).
fn entry_name(n: usize, name: Option<&str>, at: Option<DateTime<chrono::FixedOffset>>, kind: Kind) -> String {
    let mut slug = String::new();
    for ch in name.unwrap_or("").chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            slug.push(ch);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.chars().count() >= 24 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "guest" } else { slug };
    let hhmm = at.map(|t| t.format("%H%M").to_string()).unwrap_or_else(|| "0000".into());
    format!("{n:03}-{slug}-{hhmm}.{}", kind.ext())
}

/// Streams the zip to the response in 64 KB chunks from a blocking task.
struct ChanWriter {
    tx: tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
    buf: Vec<u8>,
}

const CHUNK: usize = 64 * 1024;

impl ChanWriter {
    fn send(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(std::mem::replace(&mut self.buf, Vec::with_capacity(CHUNK)));
        self.tx.blocking_send(Ok(chunk)).map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the download stopped"))
    }
}

impl Write for ChanWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(b);
        if self.buf.len() >= CHUNK {
            self.send()?;
        }
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.send()
    }
}

/// Which photos a zip holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipPick {
    Approved,
    /// Approved and waiting (`?all=1`).
    All,
    /// The best ones only (`?best=1`).
    Best,
}

/// The wall's photos as one zip (stored, not deflated: photos don't shrink). Never hidden ones.
pub async fn zip_response(state: &AppState, code: &str, pick: ZipPick) -> PResult<Response> {
    let s = super::snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    if s.row.kit != ID {
        return Err(refused(StatusCode::NOT_FOUND, "not_a_wall", "This pocket isn't a photo wall."));
    }
    let dir = wall_dir(state, code).ok_or_else(PocketError::not_found)?;
    let st = load(&s.kit_state);
    let ctx = KitCtx::new(&s.spec, &s.players, Utc::now());
    // (entry name, file, degrees a JPEG's orientation block says in the zip)
    let mut files: Vec<(String, PathBuf, u16)> = Vec::new();
    let mut total = 0u64;
    let wanted = |p: &&Photo| match pick {
        ZipPick::Approved => p.status == APPROVED,
        ZipPick::All => p.status == APPROVED || p.status == PENDING,
        ZipPick::Best => p.status == APPROVED && p.best,
    };
    for p in st.photos.iter().filter(wanted) {
        let Some(kind) = Kind::from_ext(&p.ext) else { continue };
        let name = first_name(&ctx, &p.by);
        let at = p.at.map(|t| t.with_timezone(&chrono::Local).fixed_offset());
        let turn = if kind == Kind::Jpeg { p.rotate } else { 0 };
        files.push((entry_name(files.len() + 1, name.as_deref(), at, kind), dir.join(format!("{}.{}", p.id, kind.ext())), turn));
        total += p.bytes;
    }
    let large = total > 3 * 1024 * MB;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(4);
    tokio::task::spawn_blocking(move || {
        let err_tx = tx.clone();
        let res = (|| -> zip::result::ZipResult<()> {
            let mut z = zip::ZipWriter::new_stream(ChanWriter { tx, buf: Vec::with_capacity(CHUNK) });
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(large);
            for (name, path, turn) in files {
                // A photo hidden since the list was made is gone: skip it
                let Ok(mut f) = std::fs::File::open(&path) else { continue };
                if turn != 0 {
                    // Photo apps don't know `rotate`: a JPEG gets its orientation back, in the zip only
                    let mut b = Vec::new();
                    f.read_to_end(&mut b)?;
                    z.start_file(name, opts)?;
                    z.write_all(&photo::jpeg_with_orientation(&b, turn).unwrap_or(b))?;
                    continue;
                }
                z.start_file(name, opts)?;
                std::io::copy(&mut f, &mut z)?;
            }
            let mut w = z.finish()?.into_inner();
            w.flush()?;
            Ok(())
        })();
        if let Err(e) = res {
            let _ = err_tx.blocking_send(Err(std::io::Error::other(e.to_string())));
        }
    });
    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));
    let disposition = format!("attachment; filename=\"wall-{code}.zip\"");
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("application/zip")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (header::CONTENT_DISPOSITION, HeaderValue::from_str(&disposition).map_err(|_| PocketError::internal())?),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pocket::kits::Player;

    fn players() -> Vec<Player> {
        ["Mia Rossi", "Sam"]
            .iter()
            .enumerate()
            .map(|(i, n)| Player { id: format!("pl_{i}"), name: n.to_string(), avatar: String::new(), team: None, joined_at: Utc::now() })
            .collect()
    }

    fn with_photos(statuses: &[(&str, &str)]) -> Value {
        let mut st = St { v: 1, ..St::default() };
        for (i, (by, status)) in statuses.iter().enumerate() {
            st.photos.push(Photo {
                id: format!("ph_{i}"),
                by: by.to_string(),
                at: Some(Utc::now()),
                bytes: 10,
                ext: "jpg".into(),
                status: status.to_string(),
                ..Photo::default()
            });
        }
        let mut v = Value::Null;
        store(&mut v, &st);
        v
    }

    #[test]
    fn spec_takes_only_its_fields() {
        let ok = json!({ "title": "Mia & Sam", "photo_cap_mb": 25, "pocket_cap_mb": 2048, "per_guest_max": 100, "captions": false });
        assert_eq!(Wall.validate_spec(&ok), Ok(()));
        let err = Wall.validate_spec(&json!({ "title": "x", "video": true })).unwrap_err();
        assert!(err.contains("\"video\""), "{err}");
        assert!(Wall.validate_spec(&json!({})).is_err(), "title required");
        assert!(Wall.validate_spec(&json!({ "title": "x", "photo_cap_mb": 26 })).is_err());
        assert!(Wall.validate_spec(&json!({ "title": "x", "pocket_cap_mb": 4097 })).is_err());
        assert!(Wall.validate_spec(&json!({ "title": "x", "photo_cap_mb": 20, "pocket_cap_mb": 10 })).is_err());
        assert!(Wall.validate_spec(&json!({ "title": "x", "per_guest_max": 0 })).is_err());
        assert!(Wall.validate_spec(&json!({ "title": "x", "captions": "yes" })).is_err());
        assert!(Wall.validate_spec(&json!({ "title": "x", "closes_at": "tomorrow" })).is_err());
        let c = caps(&json!({ "title": "x" }));
        assert_eq!((c.photo_bytes, c.pocket_bytes, c.per_guest, c.captions), (15 * MB, 1024 * MB, 40, true));
    }

    #[test]
    fn views_split_by_viewer() {
        let ps = players();
        let spec = json!({ "title": "W" });
        let ctx = KitCtx::new(&spec, &ps, Utc::now());
        let state = with_photos(&[("pl_0", PENDING), ("pl_1", PENDING), ("pl_0", APPROVED), ("pl_1", HIDDEN)]);
        let ids = |v: &Value, f: &str| -> Vec<String> { v[f].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap().to_string()).collect() };
        let a = Wall.view(&ctx, &state, &Viewer::Guest("pl_0".into()));
        assert_eq!(ids(&a, "wall"), ["ph_2"], "the wall is approved only");
        assert_eq!(ids(&a, "mine"), ["ph_2", "ph_0"], "A: own photos, newest first");
        assert_eq!(a["wall"][0]["by_name"], "Mia", "first name only");
        assert!(a["wall"][0].get("by").is_none(), "no player id for guests");
        assert_eq!(a["limits"]["mine_count"], 0);
        let b = Wall.view(&ctx, &state, &Viewer::Guest("pl_1".into()));
        assert_eq!(ids(&b, "mine"), ["ph_1"], "B's hidden photo is dropped, A's pending never shows");
        assert_eq!(ids(&b, "wall"), ["ph_2"]);
        let screen = Wall.view(&ctx, &state, &Viewer::BigScreen);
        assert_eq!(ids(&screen, "photos"), ["ph_2"]);
        assert_eq!(screen["host_name"], Value::Null, "no host_name in the spec: null, and the TV says \"the host\"");
        assert!(screen["photos"][0].get("by").is_none(), "no player id on the screen");
        let host = Wall.view(&ctx, &state, &Viewer::Host);
        assert_eq!(ids(&host, "queue"), ["ph_0", "ph_1"], "the queue oldest first");
        assert_eq!(host["counts"]["hidden"], 1);
        assert_eq!(host["counts"]["waiting"], 2);
        assert_eq!(host["queue"][0]["by_name"], "Mia Rossi");
    }

    #[test]
    fn lobby_has_the_limits_and_the_screen_names_the_host() {
        let ps = players();
        let spec = json!({ "title": "W", "host_name": "Oleks", "photo_cap_mb": 20, "per_guest_max": 30 });
        let ctx = KitCtx::new(&spec, &ps, Utc::now());
        let state = with_photos(&[("pl_0", PENDING), ("pl_0", APPROVED)]);
        let lobby = Wall.lobby(&ctx, &state);
        assert_eq!(lobby["limits"]["photo_cap_mb"], 20, "the join screen can say \"up to 20 MB each\"");
        assert_eq!(lobby["limits"]["per_guest_max"], 30);
        assert_eq!(lobby["limits"]["caption_max"], CAPTION_MAX);
        assert!(lobby["limits"].get("mine_count").is_none(), "no seat, so no counts of anyone's photos");
        assert_eq!(lobby["count"], 1, "approved only");
        let spec = json!({ "title": "W" });
        let dflt = Wall.lobby(&KitCtx::new(&spec, &ps, Utc::now()), &state);
        assert_eq!((dflt["limits"]["photo_cap_mb"].clone(), dflt["limits"]["per_guest_max"].clone()), (json!(PHOTO_CAP_MB), json!(PER_GUEST_MAX)), "defaults when the spec sets none");
        let screen = Wall.view(&ctx, &state, &Viewer::BigScreen);
        assert_eq!(screen["host_name"], "Oleks");
        let guest = Wall.view(&ctx, &state, &Viewer::Guest("pl_0".into()));
        assert_eq!((guest["limits"]["photo_cap_mb"].clone(), guest["limits"]["per_guest_max"].clone(), guest["limits"]["mine_count"].clone()), (json!(20), json!(30), json!(0)), "the seat's view keeps its own count");
    }

    #[test]
    fn host_actions() {
        let ps = players();
        let spec = json!({ "title": "W" });
        let mut state = with_photos(&[("pl_0", PENDING), ("pl_0", PENDING), ("pl_1", PENDING)]);
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "approve_all_from", &json!({ "player_id": "pl_0" })).unwrap();
        assert_eq!(ctx.events.len(), 2);
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "hide", &json!({ "id": "ph_2" })).unwrap();
        let st = load(&state);
        assert_eq!(st.photos[2], Photo::hidden("ph_2"), "a hidden record keeps only id + status");
        assert!(matches!(Wall.act(&mut ctx, &mut state, &Viewer::Host, "approve", &json!({ "id": "ph_2" })), Err(KitError::NotNow(_))));
        assert!(matches!(Wall.act(&mut ctx, &mut state, &Viewer::Host, "set_best", &json!({ "ids": ["ph_2"] })), Err(KitError::NotNow(_))));
        let many: Vec<String> = (0..31).map(|i| format!("ph_{i}")).collect();
        assert!(matches!(Wall.act(&mut ctx, &mut state, &Viewer::Host, "set_best", &json!({ "ids": many })), Err(KitError::BadInput(_))));
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "set_best", &json!({ "ids": ["ph_1"] })).unwrap();
        assert!(load(&state).photos[1].best && !load(&state).photos[0].best && load(&state).best_set);
        assert!(Wall.act(&mut ctx, &mut state, &Viewer::Guest("pl_0".into()), "approve", &json!({ "id": "ph_0" })).is_err());
        // A guest's caption: own photo only, and only while it waits
        let mut state = with_photos(&[("pl_0", PENDING), ("pl_1", PENDING), ("pl_0", APPROVED)]);
        let me = Viewer::Guest("pl_0".into());
        Wall.act(&mut ctx, &mut state, &me, "caption", &json!({ "id": "ph_0", "caption": "  first   dance  " })).unwrap();
        assert_eq!(load(&state).photos[0].caption.as_deref(), Some("first dance"));
        assert!(Wall.act(&mut ctx, &mut state, &me, "caption", &json!({ "id": "ph_1", "caption": "x" })).is_err(), "not B's photo");
        assert!(matches!(Wall.act(&mut ctx, &mut state, &me, "caption", &json!({ "id": "ph_2", "caption": "x" })), Err(KitError::NotNow(_))));
        Wall.act(&mut ctx, &mut state, &me, "caption", &json!({ "id": "ph_0", "caption": "what the fuck" })).unwrap();
        let p = &load(&state).photos[0];
        assert_eq!((p.caption.as_deref(), p.caption_removed), (None, true), "the filter drops it and says so");
    }

    #[test]
    fn closes_at_stops_uploads_once() {
        let ps = players();
        let spec = json!({ "title": "W", "closes_at": "2026-12-01T00:00:00Z" });
        let mut state = Wall.init(&spec);
        let before = DateTime::parse_from_rfc3339("2026-11-30T23:59:00Z").unwrap().with_timezone(&Utc);
        assert!(!Wall.tick(&mut KitCtx::new(&spec, &ps, before), &mut state));
        let after = before + chrono::Duration::minutes(2);
        assert!(Wall.tick(&mut KitCtx::new(&spec, &ps, after), &mut state));
        assert!(!Wall.tick(&mut KitCtx::new(&spec, &ps, after), &mut state), "once");
        assert_eq!(Wall.view(&KitCtx::new(&spec, &ps, after), &state, &Viewer::Host)["state"], "closed");
        assert_eq!(Wall.view(&KitCtx::new(&spec, &ps, after), &state, &Viewer::Host)["delete_on"], "2026-12-31");
    }

    #[test]
    fn captions_are_cut_and_filtered() {
        let on = json!({ "title": "W" });
        let long = "a".repeat(100);
        assert_eq!(caption_of(&on, &long).0.map(|c| c.chars().count()), Some(CAPTION_MAX));
        assert_eq!(caption_of(&on, "   "), (None, false));
        assert_eq!(header_caption(&on, Some(b"Mia%20%26%20Sam%20%F0%9F%8E%89")).unwrap(), (Some("Mia & Sam 🎉".into()), false));
        assert!(header_caption(&on, Some(b"%FF%FE")).is_err(), "not UTF-8");
        assert_eq!(caption_of(&json!({ "title": "W", "captions": false }), "hi"), (None, false));
        assert!(creature("pl_0") < 8 && creature("pl_0") == creature("pl_0"));
    }

    // ---- pa14b: the best-30 pick ----

    fn approved(n: usize) -> Value {
        let rows: Vec<(&str, &str)> = (0..n).map(|i| (if i % 2 == 0 { "pl_0" } else { "pl_1" }, APPROVED)).collect();
        with_photos(&rows)
    }

    fn picking(n: usize) -> Value {
        let mut state = approved(n);
        assert!(pick_start(&mut state));
        state
    }

    #[test]
    fn pick_step_waits_skips_or_asks() {
        // Not due yet: nothing, whatever the count
        assert_eq!(pick_step(&approved(40), false, false), PickStep::Wait);
        // Due: none approved is a skip, 30 or fewer are all proposed without a call, 31+ ask
        assert_eq!(pick_step(&approved(0), true, false), PickStep::Skip);
        let all = pick_step(&approved(30), true, false);
        assert!(matches!(&all, PickStep::ProposeAll(ids) if ids.len() == 30), "{all:?}");
        let ask = pick_step(&approved(31), true, false);
        assert!(matches!(&ask, PickStep::Ask(ids) if ids.len() == 31), "{ask:?}");
        // Pending and hidden photos don't count
        let mixed = with_photos(&[("pl_0", APPROVED), ("pl_0", PENDING), ("pl_1", HIDDEN)]);
        assert_eq!(pick_step(&mixed, true, false), PickStep::ProposeAll(vec!["ph_0".into()]));
        // A host who chose by hand is never overridden, even before it's due
        let ps = players();
        let spec = json!({ "title": "W" });
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut chose = approved(40);
        Wall.act(&mut ctx, &mut chose, &Viewer::Host, "set_best", &json!({ "ids": ["ph_3"] })).unwrap();
        assert_eq!(pick_step(&chose, true, false), PickStep::Wait, "a hand pick settles it");
        let mut st = load(&chose);
        st.pick_done = false;
        store(&mut chose, &st);
        assert_eq!(pick_step(&chose, true, false), PickStep::Skip, "and a best is a skip even if not settled");
        // Done is done
        let mut done = approved(40);
        pick_skip(&mut done);
        assert!(pick_settled(&done));
        assert_eq!(pick_step(&done, true, false), PickStep::Wait);
        // Picking: wait while it runs here; a restart's leftover asks again
        let running = picking(40);
        assert_eq!(pick_step(&running, true, true), PickStep::Wait);
        assert!(matches!(pick_step(&running, true, false), PickStep::Ask(_)));
    }

    #[test]
    fn pick_runs_at_most_twice() {
        let mut state = approved(40);
        assert!(pick_start(&mut state), "the first try");
        assert!(is_picking(&state));
        assert!(pick_start(&mut state), "once more after a restart");
        assert!(!pick_start(&mut state), "then never");
        assert!(!is_picking(&state) && pick_settled(&state));
        let st = load(&state);
        assert_eq!((st.pick_tries, st.bot_calls), (PICK_TRIES, 2));
    }

    #[test]
    fn propose_takes_only_approved_ids_of_this_wall() {
        let ps = players();
        let spec = json!({ "title": "W" });
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        // Not picking: refused
        let mut idle = approved(40);
        assert!(matches!(pick_propose(&mut ctx, &mut idle, &json!(["ph_0"])), Err(KitError::NotNow(_))));
        let mut state = picking(40);
        let mut st = load(&state);
        st.photos[5].status = PENDING.into();
        st.photos[6] = Photo::hidden("ph_6");
        store(&mut state, &st);
        fn refused(state: &mut Value, ctx: &mut KitCtx<'_>, ids: Value, needle: &str) {
            match pick_propose(ctx, state, &ids) {
                Err(KitError::BadInput(m)) => assert!(m.contains(needle), "{m}"),
                other => panic!("{ids}: {other:?}"),
            }
        }
        refused(&mut state, &mut ctx, json!("ph_0"), "list");
        refused(&mut state, &mut ctx, json!([]), "at least one");
        let many: Vec<String> = (0..31).map(|i| format!("ph_{i}")).collect();
        refused(&mut state, &mut ctx, json!(many), "At most 30");
        refused(&mut state, &mut ctx, json!(["ph_0", "ph_1", "ph_0"]), "twice");
        refused(&mut state, &mut ctx, json!(["ph_0", 7]), "text");
        refused(&mut state, &mut ctx, json!(["ph_0", "ph_999"]), "approved");
        refused(&mut state, &mut ctx, json!(["ph_5"]), "approved");
        refused(&mut state, &mut ctx, json!(["ph_6"]), "approved");
        refused(&mut state, &mut ctx, json!(["xx_other_wall"]), "approved");
        let long = format!("{}\n\u{7}", "z".repeat(500));
        match pick_propose(&mut ctx, &mut state, &json!([long])) {
            Err(KitError::BadInput(m)) => assert!(m.len() < 120 && !m.contains('\u{7}'), "{m}"),
            other => panic!("{other:?}"),
        }
        assert!(is_picking(&state) && proposed_count(&state) == 0, "a refusal leaves the pick running");
        assert!(ctx.events.is_empty());
        let ids: Vec<String> = (10..40).map(|i| format!("ph_{i}")).collect();
        assert_eq!(pick_propose(&mut ctx, &mut state, &json!(ids)).unwrap(), 30);
        assert!(!is_picking(&state) && pick_settled(&state));
        assert_eq!(load(&state).proposed_best, ids);
        assert_eq!(ctx.events.last().map(|e| e.kind.as_str()), Some("best_proposed"));
        assert!(matches!(pick_propose(&mut ctx, &mut state, &json!(["ph_0"])), Err(KitError::NotNow(_))), "once");
    }

    #[test]
    fn pick_fail_leaves_no_proposal() {
        let ps = players();
        let spec = json!({ "title": "W" });
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut state = picking(40);
        pick_fail(&mut ctx, &mut state);
        assert!(!is_picking(&state) && pick_settled(&state) && proposed_count(&state) == 0);
        assert_eq!(pick_step(&state, true, false), PickStep::Wait, "not retried");
        // The host still stars by hand, after End too
        assert_eq!(Wall.host_actions_after_end(), ["set_best"]);
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "set_best", &json!({ "ids": ["ph_1", "ph_2"] })).unwrap();
        let host = Wall.view(&ctx, &state, &Viewer::Host);
        assert_eq!((host["best_by"].clone(), host["picking"].clone()), (json!("host"), json!(false)));
    }

    #[test]
    fn best_by_is_snap_only_for_the_proposal_unchanged() {
        let ps = players();
        let spec = json!({ "title": "W" });
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut state = picking(40);
        let ids: Vec<String> = (0..30).map(|i| format!("ph_{i}")).collect();
        pick_propose(&mut ctx, &mut state, &json!(ids)).unwrap();
        let host = Wall.view(&ctx, &state, &Viewer::Host);
        assert_eq!(host["proposed_best"].as_array().map(Vec::len), Some(30));
        assert_eq!((host["best_by"].clone(), host["picking"].clone()), (Value::Null, json!(false)), "nothing shown yet");
        // A swap is the host's
        let mut swapped = ids.clone();
        swapped[0] = "ph_35".into();
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut s2 = state.clone();
        Wall.act(&mut ctx, &mut s2, &Viewer::Host, "set_best", &json!({ "ids": swapped })).unwrap();
        assert_eq!(Wall.view(&ctx, &s2, &Viewer::Host)["best_by"], "host");
        assert!(Wall.view(&ctx, &s2, &Viewer::Guest("pl_0".into())).get("best_by").is_none());
        assert!(!ctx.events.iter().any(|e| e.kind == SNAP_SHOWN_EVENT));
        // Fewer is the host's too
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut s3 = state.clone();
        Wall.act(&mut ctx, &mut s3, &Viewer::Host, "set_best", &json!({ "ids": &ids[..29] })).unwrap();
        assert_eq!(Wall.view(&ctx, &s3, &Viewer::Host)["best_by"], "host");
        // Unchanged (any order) is the bot's, and says so once
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut rev = ids.clone();
        rev.reverse();
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "set_best", &json!({ "ids": rev })).unwrap();
        assert_eq!(Wall.view(&ctx, &state, &Viewer::Host)["best_by"], "snap");
        assert_eq!(Wall.view(&ctx, &state, &Viewer::Guest("pl_0".into()))["best_by"], "snap");
        assert_eq!(ctx.events.iter().filter(|e| e.kind == SNAP_SHOWN_EVENT).count(), 1);
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "set_best", &json!({ "ids": ids })).unwrap();
        assert_eq!(ctx.events.iter().filter(|e| e.kind == SNAP_SHOWN_EVENT).count(), 1, "said once");
        // Then a swap makes it the host's
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "set_best", &json!({ "ids": swapped })).unwrap();
        assert_eq!(Wall.view(&ctx, &state, &Viewer::Host)["best_by"], "host");
    }

    #[test]
    fn all_of_thirty_or_fewer_unchanged_is_the_hosts_not_the_bots() {
        // copy check (Mira, 3032d327): no bot picked anything, so no holo sticker, no state-8 line
        let ps = players();
        let spec = json!({ "title": "W" });
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut state = approved(12);
        let ids: Vec<String> = (0..12).map(|i| format!("ph_{i}")).collect();
        pick_propose_all(&mut ctx, &mut state, &ids);
        assert!(!load(&state).proposed_by_bot);
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "set_best", &json!({ "ids": ids })).unwrap();
        assert_eq!(Wall.view(&ctx, &state, &Viewer::Host)["best_by"], "host");
        assert!(Wall.view(&ctx, &state, &Viewer::Guest("pl_0".into())).get("best_by").is_none());
        assert!(!ctx.events.iter().any(|e| e.kind == SNAP_SHOWN_EVENT), "no state-8 line");
        // More than 30, the bot's 30 unchanged: still the bot's
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut state = picking(40);
        let ids: Vec<String> = (0..30).map(|i| format!("ph_{i}")).collect();
        pick_propose(&mut ctx, &mut state, &json!(ids)).unwrap();
        assert!(load(&state).proposed_by_bot);
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "set_best", &json!({ "ids": ids })).unwrap();
        assert_eq!(Wall.view(&ctx, &state, &Viewer::Host)["best_by"], "snap");
        assert_eq!(ctx.events.iter().filter(|e| e.kind == SNAP_SHOWN_EVENT).count(), 1);
    }

    #[test]
    fn the_proposal_is_for_the_host_only() {
        let ps = players();
        let spec = json!({ "title": "W" });
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut state = picking(40);
        let guest = Wall.view(&ctx, &state, &Viewer::Guest("pl_0".into()));
        assert!(guest.get("picking").is_none());
        let ids: Vec<String> = (0..30).map(|i| format!("ph_{i}")).collect();
        pick_propose(&mut ctx, &mut state, &json!(ids)).unwrap();
        for (who, v) in [
            ("guest", Wall.view(&ctx, &state, &Viewer::Guest("pl_0".into()))),
            ("screen", Wall.view(&ctx, &state, &Viewer::BigScreen)),
            ("lobby", Wall.lobby(&ctx, &state)),
        ] {
            let text = v.to_string();
            for key in ["proposed_best", "picking", "budget", "bot_calls"] {
                assert!(!text.contains(key), "{who} has {key}: {text}");
            }
            assert!(!text.contains("\"best\":true"), "{who}: nothing is best until the host says yes");
        }
        let summary = Wall.host_summary(&ctx, &state);
        assert_eq!(summary["proposed_best"].as_array().map(Vec::len), Some(30));
        assert_eq!(summary["budget"]["bot_calls"], 1);
    }

    #[test]
    fn hiding_the_last_best_photo_takes_the_best_away() {
        let ps = players();
        let spec = json!({ "title": "W" });
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut state = picking(40);
        let ids: Vec<String> = (0..2).map(|i| format!("ph_{i}")).collect();
        pick_propose(&mut ctx, &mut state, &json!(ids)).unwrap();
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "set_best", &json!({ "ids": ids })).unwrap();
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "hide", &json!({ "id": "ph_0" })).unwrap();
        let st = load(&state);
        assert!(st.best_set && st.best_by_snap, "one best photo left");
        assert_eq!(st.proposed_best, ["ph_1"], "the hidden one leaves the proposal");
        Wall.act(&mut ctx, &mut state, &Viewer::Host, "hide", &json!({ "id": "ph_1" })).unwrap();
        let st = load(&state);
        assert!(!st.best_set && !st.best_by_snap, "F4: no best photo, so no best for guests");
        let guest = Wall.view(&ctx, &state, &Viewer::Guest("pl_0".into()));
        assert_eq!(guest["best_set"], false);
        assert_eq!(Wall.view(&ctx, &state, &Viewer::Host)["best_by"], Value::Null);
    }

    #[test]
    fn budget_line() {
        let ps = players();
        let spec = json!({ "title": "W" });
        let ctx = KitCtx::new(&spec, &ps, Utc::now());
        let none = Wall.host_summary(&ctx, &approved(3));
        assert_eq!(none["budget"]["bot_calls"], 0);
        assert!(none["budget"]["line"].as_str().unwrap().starts_with("No bot calls so far."));
        let one = Wall.host_summary(&ctx, &picking(40));
        assert_eq!(one["budget"]["line"], "1 bot call so far, about $0.10 on your account, to pick the best 30.");
        let mut two = picking(40);
        assert!(pick_start(&mut two));
        assert_eq!(Wall.host_summary(&ctx, &two)["budget"]["usd"], 0.2);
    }

    #[test]
    fn zip_names() {
        let at = DateTime::parse_from_rfc3339("2026-12-01T21:14:00+00:00").unwrap();
        assert_eq!(entry_name(1, Some("Mia"), Some(at), Kind::Jpeg), "001-mia-2114.jpg");
        assert_eq!(entry_name(12, Some("Zoë-Ann  O'Neil"), Some(at), Kind::Png), "012-zoë-ann-o-neil-2114.png");
        assert_eq!(entry_name(3, None, Some(at), Kind::Webp), "003-guest-2114.webp");
        assert_eq!(entry_name(4, Some("../.."), None, Kind::Jpeg), "004-guest-0000.jpg");
    }
}
