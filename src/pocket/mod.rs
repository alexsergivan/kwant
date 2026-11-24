//! Pocket apps core (pa1, Pocket API v1). A pocket is a kit (a tested engine kwant ships, see
//! [`kits`]) plus a spec the bot wrote. Guests join from their phones with a name (and a PIN when
//! the kit asks), get a key in a cookie scoped to that one pocket, and play over WebSocket or by
//! polling. Guests reach only the separate guest listener ([`listener`], [`guest`]); the person's
//! main port has only the person-only list/read/end/delete API ([`host_api`]).
//!
//! Everything is behind the `pocket_apps` setting (`on`); while it is off no listener runs, the
//! sweeper does nothing and the main port's list is empty.
//!
//! Every change to a pocket runs under that pocket's lock: read, let the kit act, save the kit's
//! state with a new version, refresh the in-memory snapshot, and signal the pocket's viewers,
//! each of whom then draws its own view from the snapshot.

pub mod address;
pub mod desk;
pub mod gather;
pub mod guest;
pub mod host_api;
pub mod keys;
pub mod kits;
pub mod limits;
pub mod listener;
pub mod photo;
pub mod quiz_kit;
pub mod santa;
pub mod schedule;
pub mod tunnel;
pub mod wall;
pub mod words;

use crate::config::Config;
use crate::db::pocket::{NewPlayer, NewPocket, PinCounters, PlayerRow, PocketRow, ENDED, OPEN};
use crate::db::Database;
use crate::server::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use kits::{Kit, KitCtx, KitError, KitRules, Player, Viewer};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::broadcast;

/// The switch: pocket apps run only while this setting is `on`.
pub const SETTING: &str = "pocket_apps";
/// The version every guest response names (`"api": "pocket/1"`).
pub const API_VERSION: &str = "pocket/1";
/// Wrong PIN tries before a 10-minute wait.
pub const PIN_TRIES: i64 = 5;
pub const PIN_WAIT_MINUTES: i64 = 10;
/// Wrong PIN tries in total before the name locks until the host makes a reset code.
pub const PIN_HARD_LOCK: i64 = 10;
pub const RESET_CODE_HOURS: i64 = 24;
/// The biggest spec a pocket takes (a 40-question quiz is far smaller).
pub const MAX_SPEC_BYTES: usize = 256 * 1024;
/// Polling clients count as watching for this long after their last request.
const ACTIVE_SECS: u64 = 10;

pub async fn enabled(db: &Database) -> bool {
    matches!(db.get_setting(SETTING).await, Ok(Some(v)) if v.trim() == "on")
}

/// The guest port: `pocket_port` in the config (env `KWNT_POCKET_PORT`), else the main port + 1.
pub fn pocket_port(config: &Config) -> Option<u16> {
    config.pocket_port.or_else(|| config.port.checked_add(1))
}

/// What changed, for the pocket's open WebSockets.
#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    Changed(i64),
    /// This player's keys stopped working (removed, or the host reset the PIN); `host` for the host
    /// key (and host links); `link` when an old link session was dropped for a newer one.
    Revoked(String),
    Deleted,
}

/// A pocket as viewers see it right now, rebuilt after every change.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub row: PocketRow,
    pub spec: Value,
    pub kit_state: Value,
    pub rows: Vec<PlayerRow>,
    pub players: Vec<Player>,
}

impl Snapshot {
    pub fn kit(&self) -> Option<&'static dyn Kit> {
        kits::kit(&self.row.kit)
    }

    pub fn rules(&self) -> Option<KitRules> {
        self.kit().map(|k| effective_rules(k, &self.spec))
    }
}

/// The in-memory side of pockets: per-pocket locks, viewers' channels, snapshots, limiters and
/// the listener. Nothing here is written to disk.
#[derive(Default)]
pub struct Hub {
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    channels: Mutex<HashMap<String, broadcast::Sender<Signal>>>,
    snapshots: Mutex<HashMap<String, Arc<Snapshot>>>,
    active: Mutex<HashMap<String, Instant>>,
    pub limits: limits::Limits,
    /// Mirrors the switch for the guest listener's requests (set by [`listener::reconcile`]).
    pub on: AtomicBool,
    pub listener: listener::ListenerSlot,
    /// The box's one quick tunnel for join from anywhere (pa6), shared by the pockets that asked.
    pub tunnel: tunnel::Tunnels,
    /// macOS: `caffeinate` held while a tunnel or an open pocket runs.
    pub awake: tunnel::KeepAwake,
    /// Tests only: the free disk space the Wall's upload check sees (pa14), instead of the disk's.
    fake_free_disk: Mutex<Option<u64>>,
}

impl Hub {
    fn lock_for(&self, code: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks.lock().unwrap_or_else(|e| e.into_inner()).entry(code.to_string()).or_default().clone()
    }

    pub fn subscribe(&self, code: &str) -> broadcast::Receiver<Signal> {
        self.channels.lock().unwrap_or_else(|e| e.into_inner()).entry(code.to_string()).or_insert_with(|| broadcast::channel(64).0).subscribe()
    }

    fn signal(&self, code: &str, s: Signal) {
        if let Some(tx) = self.channels.lock().unwrap_or_else(|e| e.into_inner()).get(code) {
            let _ = tx.send(s);
        }
    }

    /// Tests only: make the Wall's free-disk check see `bytes` (`None`: the real disk again).
    pub fn set_fake_free_disk(&self, bytes: Option<u64>) {
        *self.fake_free_disk.lock().unwrap_or_else(|e| e.into_inner()) = bytes;
    }

    fn fake_free_disk(&self) -> Option<u64> {
        *self.fake_free_disk.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn touch(&self, code: &str) {
        self.active.lock().unwrap_or_else(|e| e.into_inner()).insert(code.to_string(), Instant::now());
    }

    /// Pockets someone is watching: an open WebSocket, or a request in the last 10 s.
    pub fn watched(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .channels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, tx)| tx.receiver_count() > 0)
            .map(|(c, _)| c.clone())
            .collect();
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        active.retain(|_, t| t.elapsed().as_secs() < ACTIVE_SECS);
        out.extend(active.keys().cloned());
        out.sort();
        out.dedup();
        out
    }

    fn forget(&self, code: &str) {
        self.snapshots.lock().unwrap_or_else(|e| e.into_inner()).remove(code);
        self.channels.lock().unwrap_or_else(|e| e.into_inner()).remove(code);
        self.active.lock().unwrap_or_else(|e| e.into_inner()).remove(code);
        self.locks.lock().unwrap_or_else(|e| e.into_inner()).remove(code);
    }
}

/// A refusal with its HTTP status, a stable `error` code for pages, and plain words.
#[derive(Debug, Clone, PartialEq)]
pub struct PocketError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub extra: Value,
}

impl PocketError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), extra: json!({}) }
    }

    fn with(mut self, extra: Value) -> Self {
        self.extra = extra;
        self
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "There's no pocket app here.")
    }

    fn internal() -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Something went wrong on the host's computer. Try again.")
    }
}

impl From<crate::db::DbError> for PocketError {
    fn from(e: crate::db::DbError) -> Self {
        // The error names no guest text: sqlx errors carry SQL and codes, never bound values
        tracing::warn!("pocket apps: database error: {e}");
        Self::internal()
    }
}

impl IntoResponse for PocketError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.code, "message": self.message });
        if let (Some(b), Some(extra)) = (body.as_object_mut(), self.extra.as_object()) {
            for (k, v) in extra {
                b.insert(k.clone(), v.clone());
            }
        }
        (self.status, Json(body)).into_response()
    }
}

pub type PResult<T> = Result<T, PocketError>;

fn effective_rules(kit: &dyn Kit, spec: &Value) -> KitRules {
    let mut r = kit.rules(spec);
    r.max_players = r.max_players.clamp(1, kits::DEFAULT_MAX_PLAYERS);
    r
}

fn to_player(r: &PlayerRow) -> Player {
    Player { id: r.id.clone(), name: r.name.clone(), avatar: r.avatar.clone(), team: r.team.clone(), joined_at: r.joined_at }
}

/// The pocket's snapshot, loaded from the database when it isn't cached. `None`: no such pocket.
pub async fn snapshot(state: &AppState, code: &str) -> PResult<Option<Arc<Snapshot>>> {
    if let Some(s) = state.pocket.snapshots.lock().unwrap_or_else(|e| e.into_inner()).get(code) {
        return Ok(Some(s.clone()));
    }
    refresh(state, code).await
}

async fn refresh(state: &AppState, code: &str) -> PResult<Option<Arc<Snapshot>>> {
    let Some(row) = state.db.pocket_get(code).await? else {
        state.pocket.snapshots.lock().unwrap_or_else(|e| e.into_inner()).remove(code);
        return Ok(None);
    };
    let rows = state.db.pocket_players(code).await?;
    let snap = Arc::new(Snapshot {
        spec: serde_json::from_str(&row.spec).unwrap_or(Value::Null),
        kit_state: serde_json::from_str(&row.kit_state).unwrap_or(Value::Null),
        players: rows.iter().map(to_player).collect(),
        rows,
        row,
    });
    state.pocket.snapshots.lock().unwrap_or_else(|e| e.into_inner()).insert(code.to_string(), snap.clone());
    Ok(Some(snap))
}

/// After a change: reload the snapshot and tell the viewers.
async fn changed(state: &AppState, code: &str) -> PResult<()> {
    if let Some(s) = refresh(state, code).await? {
        state.pocket.signal(code, Signal::Changed(s.row.version));
    }
    Ok(())
}

/// The public facts about a pocket (no names, no guest text): for the join page and the card.
pub fn pocket_json(s: &Snapshot) -> Value {
    let rules = s.rules();
    // pa13a: a front desk guest never learns how many other guests there are
    let private = s.kit().is_some_and(|k| k.private_guests());
    let mut out = json!({
        "code": s.row.code,
        "kit": s.row.kit,
        "title": s.row.title,
        "state": s.row.state,
        "opens_at": s.row.opens_at,
        "closes_at": s.row.closes_at,
        "ended_at": s.row.ended_at,
        "expires_at": s.row.expires_at,
        "players": s.players.len(),
        "max_players": rules.as_ref().map(|r| r.max_players),
        "needs_pin": rules.as_ref().is_some_and(|r| r.needs_pin),
        "retention": rules.as_ref().map(|r| r.retention.describe()),
    });
    if private {
        out["players"] = Value::Null;
        out["max_players"] = Value::Null;
    }
    out
}

/// What one viewer gets, the same over `state` and `ws`: `{api, version, pocket, me, view}`.
pub fn view_json(s: &Snapshot, viewer: &Viewer, now: DateTime<Utc>) -> Value {
    let Some(kit) = s.kit() else {
        return json!({ "api": API_VERSION, "version": s.row.version, "pocket": pocket_json(s), "me": null, "view": null });
    };
    let ctx = KitCtx::new(&s.spec, &s.players, now);
    let me = viewer.player_id().and_then(|id| s.rows.iter().find(|r| r.id == id)).map(|r| {
        let mut notices = Vec::new();
        if let Some(at) = r.last_wrong_at {
            notices.push(json!({ "kind": "wrong_pin", "at": at }));
        }
        json!({ "id": r.id, "name": r.name, "avatar": r.avatar, "team": r.team, "notices": notices })
    });
    let role = match viewer {
        Viewer::Guest(_) => "guest",
        Viewer::Host => "host",
        Viewer::BigScreen => "screen",
    };
    json!({
        "api": API_VERSION,
        "version": s.row.version,
        "viewer": role,
        "pocket": pocket_json(s),
        "me": me,
        "view": kit.view(&ctx, &s.kit_state, viewer),
    })
}

/// pa13a: one part of the host's view by key (`guest:<id>`: a front desk conversation), for the
/// remote and the person's card: `{api, version, ...the kit's part}`; 404 when there's no such part.
pub fn host_detail_json(s: &Snapshot, key: &str, now: DateTime<Utc>) -> PResult<Value> {
    let kit = s.kit().ok_or_else(PocketError::not_found)?;
    let ctx = KitCtx::new(&s.spec, &s.players, now);
    let mut part = kit
        .host_detail(&ctx, &s.kit_state, key)
        .ok_or_else(|| PocketError::new(StatusCode::NOT_FOUND, "no_such_player", "That player isn't in this pocket."))?;
    if let Some(o) = part.as_object_mut() {
        o.insert("api".into(), json!(API_VERSION));
        o.insert("version".into(), json!(s.row.version));
    }
    Ok(part)
}

// ---- Creating and ending (the person's side) ----

/// What the bot or the person asks for (`POST /api/pockets`, and pa3's tools).
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct CreateRequest {
    pub kit: String,
    pub spec: Value,
    #[serde(default)]
    pub owner_kwant_id: Option<String>,
    #[serde(default)]
    pub opens_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub closes_at: Option<DateTime<Utc>>,
    /// The chat the pocket came from (pa7): its reminder and results are posted there. Without
    /// it, the person gets the notifications only.
    #[serde(default)]
    pub origin: Option<schedule::Origin>,
}

/// Makes a pocket; returns it and its host key (shown once, kept only as a hash).
pub async fn create(state: &AppState, mut req: CreateRequest) -> PResult<(Arc<Snapshot>, String)> {
    let kit = kits::kit(&req.kit).ok_or_else(|| PocketError::new(StatusCode::BAD_REQUEST, "unknown_kit", format!("There's no kit called {:?}.", req.kit)))?;
    if kit.answer_bot() {
        fill_answer_bot(state, &mut req).await?;
    }
    let spec_text = serde_json::to_string(&req.spec).map_err(|_| PocketError::internal())?;
    if spec_text.len() > MAX_SPEC_BYTES {
        return Err(PocketError::new(StatusCode::PAYLOAD_TOO_LARGE, "spec_too_big", "The spec is bigger than 256 KB."));
    }
    kit.validate_spec(&req.spec).map_err(|m| PocketError::new(StatusCode::BAD_REQUEST, "bad_spec", m))?;
    if let (Some(o), Some(c)) = (req.opens_at, req.closes_at) {
        if c <= o {
            return Err(PocketError::new(StatusCode::BAD_REQUEST, "bad_times", "closes_at must be after opens_at."));
        }
    }
    if let Some(o) = &req.origin {
        schedule::check_origin(state, o).await?;
    }
    let now = Utc::now();
    // pa7: a future `when` sets the times the bot didn't (lobby 30 min before, close 4 h after)
    let (times, mut sched) = schedule::plan(state, &req.spec, req.opens_at, req.closes_at, req.origin.as_ref(), now).await;
    if let (Some(o), Some(c)) = (times.opens_at, times.closes_at) {
        if c <= o {
            return Err(PocketError::new(StatusCode::BAD_REQUEST, "bad_times", "closes_at must be after opens_at."));
        }
    }
    let rules = effective_rules(kit, &req.spec);
    let host_key = keys::new_key();
    let kit_state = serde_json::to_string(&kit.init(&req.spec)).map_err(|_| PocketError::internal())?;
    let title = kit.title(&req.spec);
    for _ in 0..8 {
        let code = keys::new_code();
        let res = state
            .db
            .pocket_insert(NewPocket {
                code: &code,
                kit: kit.id(),
                title: &title,
                spec: &spec_text,
                kit_state: &kit_state,
                owner_kwant_id: req.owner_kwant_id.as_deref(),
                opens_at: times.opens_at,
                closes_at: times.closes_at,
                expires_at: rules.retention.expires_at(&req.spec, None),
                host_key_hash: &keys::hash_key(&host_key),
                now,
            })
            .await;
        match res {
            Ok(()) => {
                sched.pocket_code = code.clone();
                state.db.pocket_schedule_insert(&sched).await?;
                let snap = refresh(state, &code).await?.ok_or_else(PocketError::internal)?;
                return Ok((snap, host_key));
            }
            Err(crate::db::DbError::Sqlx(sqlx::Error::Database(e))) if e.is_unique_violation() => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(PocketError::internal())
}

/// pa13a: a kit whose guests a bot answers (the front desk) needs that bot: the spec's
/// `answer_bot`, else the bot that made the pocket. The spec then holds its id and its face
/// (`bot: {name, sprite}`) for the views; guests never see the id.
async fn fill_answer_bot(state: &AppState, req: &mut CreateRequest) -> PResult<()> {
    let id = req.spec.get("answer_bot").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).or_else(|| req.owner_kwant_id.clone());
    let needs_bot = || PocketError::new(StatusCode::BAD_REQUEST, "needs_bot", "Pick the bot that answers guests (answer_bot).");
    let id = id.ok_or_else(needs_bot)?;
    let bot = state.db.get_kwant(&id).await?.ok_or_else(needs_bot)?;
    if let Some(spec) = req.spec.as_object_mut() {
        spec.insert("answer_bot".into(), json!(bot.id));
        spec.insert("bot".into(), json!({ "name": bot.name, "sprite": desk::SPRITE }));
    }
    Ok(())
}

/// Ends an open pocket and sets when its data goes (from the kit's retention).
pub async fn end(state: &AppState, code: &str) -> PResult<Arc<Snapshot>> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    if s.row.state == OPEN {
        let now = Utc::now();
        let expires = s.rules().map_or(Some(now), |r| r.retention.expires_at(&s.spec, Some(now)));
        state.db.pocket_end(code, now, expires).await?;
        changed(state, code).await?;
    }
    state.pocket.tunnel.release(code);
    snapshot(state, code).await?.ok_or_else(PocketError::not_found)
}

/// Join from anywhere on or off for one pocket (pa6). On: joins the box's quick tunnel or starts
/// it in the background; refused (409) for a long-lived kit, an ended pocket or a closed guest
/// port. Off: this pocket stops using it, and the last user stops it.
pub async fn set_tunnel(state: &AppState, code: &str, on: bool) -> PResult<()> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    if !on {
        state.pocket.tunnel.release(code);
        return Ok(());
    }
    if let Some((why, line)) = tunnel::refusal(&s) {
        return Err(PocketError::new(StatusCode::CONFLICT, why, line));
    }
    let Some(guest) = state.pocket.listener.addr() else {
        return Err(PocketError::new(StatusCode::CONFLICT, "guest_port_closed", "The guest page isn't running yet. Try again in a few seconds."));
    };
    tunnel::turn_on(state, code, guest.port());
    Ok(())
}

/// Deletes the pocket and everything about its guests now. Final.
pub async fn delete(state: &AppState, code: &str) -> PResult<bool> {
    let lock = state.pocket.lock_for(code);
    let gone = {
        let _g = lock.lock().await;
        // pa13a: the bot's conversations with the guests go with the pocket
        let threads = snapshot(state, code).await?.and_then(|s| s.kit().map(|k| k.owned_threads(&s.kit_state))).unwrap_or_default();
        let gone = state.db.pocket_delete(code).await?;
        drop_threads(state, &threads).await;
        // pa14: the Wall's photo files go with the pocket
        wall::remove_files(state, code).await;
        gone
    };
    state.pocket.tunnel.release(code);
    state.pocket.signal(code, Signal::Deleted);
    state.pocket.forget(code);
    Ok(gone)
}

/// pa13a: deletes kwant threads a kit no longer points at (a guest's conversation with the bot).
async fn drop_threads(state: &AppState, threads: &[String]) {
    for t in threads {
        if let Err(e) = state.db.delete_thread_with_messages(t).await {
            tracing::warn!("pocket apps: couldn't delete a guest's conversation: {e}");
        }
    }
}

/// Threads in `before` but not in `after`.
fn threads_gone(kit: &dyn Kit, before: &Value, after: &Value) -> Vec<String> {
    let kept = kit.owned_threads(after);
    kit.owned_threads(before).into_iter().filter(|t| !kept.contains(t)).collect()
}

/// A new host key for the remote; the old one stops working at once, and so do host links
/// (unspent tokens and signed-in remotes). Screen links stay: a rotation never blanks the TV.
pub async fn rotate_host_key(state: &AppState, code: &str) -> PResult<String> {
    let key = keys::new_key();
    if !state.db.pocket_set_host_key(code, &keys::hash_key(&key)).await? {
        return Err(PocketError::not_found());
    }
    state.db.pocket_link_drop_host(code).await?;
    refresh(state, code).await?;
    state.pocket.signal(code, Signal::Revoked("host".into()));
    Ok(key)
}

// ---- Host and screen links (pa1b) ----

/// How long a host or screen link's token works before it is spent.
pub const LINK_MINUTES: i64 = 10;

/// What a signed-in remote or screen may do: `Host` everything the host key does, `Screen` only
/// read the big screen's view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRole {
    Host,
    Screen,
}

impl HostRole {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "host" => Some(Self::Host),
            "screen" => Some(Self::Screen),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Screen => "screen",
        }
    }
}

pub fn link_used_or_expired() -> PocketError {
    PocketError::new(StatusCode::UNAUTHORIZED, "link_used_or_expired", "This link was already used or is too old. Open a new one from the card.")
}

/// A one-time sign-in token for `role` (the person's card asks for it). Nothing is revoked.
pub async fn mint_link(state: &AppState, code: &str, role: HostRole, now: DateTime<Utc>) -> PResult<(String, DateTime<Utc>)> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    if state.db.pocket_get(code).await?.is_none() {
        return Err(PocketError::not_found());
    }
    let token = keys::new_key();
    let expires_at = now + Duration::minutes(LINK_MINUTES);
    state.db.pocket_link_token_insert(code, role.as_str(), &keys::hash_key(&token), now, expires_at).await?;
    Ok((token, expires_at))
}

/// Spends a link token of this pocket: its role and a new session key for the cookie. A spent,
/// expired or other pocket's token is [`link_used_or_expired`].
pub async fn spend_link(state: &AppState, code: &str, token: &str, now: DateTime<Utc>) -> PResult<(HostRole, String)> {
    let session = keys::new_key();
    let spent = {
        let lock = state.pocket.lock_for(code);
        let _g = lock.lock().await;
        if state.db.pocket_get(code).await?.is_none() {
            return Err(PocketError::not_found());
        }
        state.db.pocket_link_spend(code, &keys::hash_key(token), &keys::hash_key(&session), now).await?
    };
    let (role, dropped) = spent.ok_or_else(link_used_or_expired)?;
    let role = HostRole::parse(&role).ok_or_else(PocketError::internal)?;
    if dropped {
        // The oldest session of this role went to make room: its socket checks again
        state.pocket.signal(code, Signal::Revoked("link".into()));
    }
    Ok((role, session))
}

/// What `key` opens on this pocket: the host key or a host link session is `Host`, a screen
/// link session is `Screen`; anything else is `None`.
pub async fn host_role(state: &AppState, s: &Snapshot, key: &str) -> PResult<Option<HostRole>> {
    if is_host_key(s, key) {
        return Ok(Some(HostRole::Host));
    }
    Ok(state.db.pocket_link_session_role(&s.row.code, &keys::hash_key(key)).await?.as_deref().and_then(HostRole::parse))
}

/// Is `key` this pocket's host key?
pub fn is_host_key(s: &Snapshot, key: &str) -> bool {
    let got = keys::hash_key(key);
    got.len() == s.row.host_key_hash.len() && got.bytes().zip(s.row.host_key_hash.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
}

/// The player `key` signs in as, in this pocket only.
pub async fn guest_for_key(state: &AppState, code: &str, key: &str) -> PResult<Option<PlayerRow>> {
    Ok(state.db.pocket_key_player(code, &keys::hash_key(key)).await?)
}

// ---- Guests ----

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct JoinRequest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub pin: Option<String>,
}

fn clean_avatar(a: Option<&str>) -> PResult<String> {
    let a = a.unwrap_or("").trim();
    if a.len() > 32 || !a.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
        return Err(PocketError::new(StatusCode::BAD_REQUEST, "bad_avatar", "Pick one of the creatures."));
    }
    Ok(a.to_string())
}

fn clean_team(t: Option<&str>) -> PResult<Option<String>> {
    match t.map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(None),
        Some(t) => match words::clean_name(t) {
            Ok((shown, _)) => Ok(Some(shown)),
            Err(e) => Err(PocketError::new(StatusCode::BAD_REQUEST, "bad_team", e.message())),
        },
    }
}

fn open_check(s: &Snapshot, now: DateTime<Utc>) -> PResult<()> {
    if s.row.state == ENDED {
        return Err(PocketError::new(StatusCode::GONE, "ended", "This pocket app has ended."));
    }
    if s.row.opens_at.is_some_and(|o| now < o) {
        return Err(PocketError::new(StatusCode::CONFLICT, "not_open_yet", "It isn't open yet.").with(json!({ "opens_at": s.row.opens_at })));
    }
    Ok(())
}

/// A new player: returns the player and the guest key for the cookie.
pub async fn join(state: &AppState, code: &str, req: JoinRequest) -> PResult<(PlayerRow, String)> {
    let (name, name_key) = words::clean_name(&req.name).map_err(|e| PocketError::new(StatusCode::BAD_REQUEST, e.code(), e.message()))?;
    let avatar = clean_avatar(req.avatar.as_deref())?;
    let team = clean_team(req.team.as_deref())?;
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    let kit = s.kit().ok_or_else(PocketError::not_found)?;
    let rules = effective_rules(kit, &s.spec);
    let now = Utc::now();
    open_check(&s, now)?;
    let pin = match (rules.needs_pin, req.pin.as_deref()) {
        (true, Some(p)) if keys::valid_pin(p) => Some(p),
        (true, _) => return Err(PocketError::new(StatusCode::BAD_REQUEST, "pin_required", "Pick a 4-digit PIN.")),
        (false, _) => None,
    };
    if let Some(existing) = state.db.pocket_player_by_name(code, &name_key).await? {
        return Err(if existing.removed_at.is_some() {
            PocketError::new(StatusCode::FORBIDDEN, "removed", "The host removed this name.")
        } else {
            PocketError::new(StatusCode::CONFLICT, "name_taken", "Someone already joined with that name. If it's you, open your page again.")
        });
    }
    if s.players.len() >= rules.max_players {
        return Err(PocketError::new(StatusCode::CONFLICT, "full", "It's full.").with(json!({ "max_players": rules.max_players })));
    }
    kit.admit(&KitCtx::new(&s.spec, &s.players, now), &s.kit_state, &name_key).map_err(kit_refusal)?;
    let id = keys::new_player_id();
    let salt = keys::new_salt();
    let pin_hash = pin.map(|p| keys::hash_secret(p, &salt));
    state
        .db
        .pocket_player_insert(NewPlayer {
            pocket_code: code,
            id: &id,
            name: &name,
            name_key: &name_key,
            avatar: &avatar,
            team: team.as_deref(),
            pin_hash: pin_hash.as_deref(),
            pin_salt: Some(&salt),
            now,
        })
        .await?;
    let key = keys::new_key();
    state.db.pocket_key_insert(code, &id, &keys::hash_key(&key), now).await?;
    // The kit sees the new player in its list
    let mut players = s.players.clone();
    let row = state.db.pocket_player(code, &id).await?.ok_or_else(PocketError::internal)?;
    players.push(to_player(&row));
    let mut kit_state = s.kit_state.clone();
    let mut ctx = KitCtx::new(&s.spec, &players, now);
    kit.on_join(&mut ctx, &mut kit_state, &to_player(&row));
    save(state, code, &kit_state, ctx, now).await?;
    Ok((row, key))
}

async fn save(state: &AppState, code: &str, kit_state: &Value, ctx: KitCtx<'_>, now: DateTime<Utc>) -> PResult<()> {
    let events: Vec<(Option<String>, String, String)> =
        ctx.events.into_iter().map(|e| (e.player_id, e.kind, serde_json::to_string(&e.payload).unwrap_or_else(|_| "{}".into()))).collect();
    let text = serde_json::to_string(kit_state).map_err(|_| PocketError::internal())?;
    state.db.pocket_save_state(code, &text, &events, now).await?;
    changed(state, code).await?;
    // pa14: a photo the save hid loses its file now
    if let Some(s) = snapshot(state, code).await?.filter(|s| s.row.kit == wall::ID) {
        wall::prune(state, &s).await;
    }
    Ok(())
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct RejoinRequest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub pin: Option<String>,
    /// The host's one-time code, with the new PIN to set.
    #[serde(default)]
    pub reset_code: Option<String>,
    #[serde(default)]
    pub new_pin: Option<String>,
}

/// A player opens their page again (new phone, cookie gone). Returns the player, a new key and
/// the notices they haven't seen (a wrong PIN try, the host's reset).
pub async fn rejoin(state: &AppState, code: &str, req: RejoinRequest) -> PResult<(PlayerRow, String, Vec<Value>)> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    let rules = s.rules().ok_or_else(PocketError::not_found)?;
    let now = Utc::now();
    let name_key = words::clean_name(&req.name).map(|(_, k)| k).map_err(|_| no_such_name())?;
    let p = state.db.pocket_player_by_name(code, &name_key).await?.ok_or_else(no_such_name)?;
    if p.removed_at.is_some() {
        return Err(PocketError::new(StatusCode::FORBIDDEN, "removed", "The host removed this name."));
    }
    let mut notices = Vec::new();
    if rules.needs_pin {
        if p.pin_locked {
            return Err(hard_locked());
        }
        if let Some(until) = p.locked_until.filter(|u| *u > now) {
            return Err(PocketError::new(StatusCode::LOCKED, "locked", "Too many wrong PINs. Wait 10 minutes, then try again.")
                .with(json!({ "until": until, "tries_left": 0 })));
        }
        let salt = p.pin_salt.clone().unwrap_or_default();
        let new_pin_hash = match req.reset_code.as_deref() {
            Some(code_typed) => {
                let (Some(hash), Some(expires)) = (p.reset_code_hash.as_deref(), p.reset_expires_at) else {
                    return Err(PocketError::new(StatusCode::CONFLICT, "no_reset", "There's no reset code waiting for this name. Use your PIN."));
                };
                if expires <= now {
                    return Err(PocketError::new(StatusCode::GONE, "reset_expired", "That reset code has expired. Ask the host for a new one."));
                }
                let new_pin = req.new_pin.as_deref().filter(|n| keys::valid_pin(n)).ok_or_else(|| PocketError::new(StatusCode::BAD_REQUEST, "pin_required", "Pick a new 4-digit PIN."))?;
                if !keys::secret_matches(&keys::normalize_reset_code(code_typed), &salt, hash) {
                    return Err(wrong_try(state, code, &p, now).await?);
                }
                let new_salt = keys::new_salt();
                Some((keys::hash_secret(new_pin, &new_salt), new_salt))
            }
            None => {
                let Some(hash) = p.pin_hash.as_deref() else {
                    return Err(PocketError::new(StatusCode::CONFLICT, "reset_pending", "The host made you a reset code. Use it to pick a new PIN."));
                };
                let pin = req.pin.as_deref().unwrap_or("");
                if !keys::valid_pin(pin) || !keys::secret_matches(pin, &salt, hash) {
                    return Err(wrong_try(state, code, &p, now).await?);
                }
                None
            }
        };
        if let Some(at) = p.last_wrong_at {
            notices.push(json!({ "kind": "wrong_pin", "at": at }));
        }
        if let Some(at) = p.pin_reset_at {
            notices.push(json!({ "kind": "pin_reset", "at": at }));
        }
        state.db.pocket_pin_ok(code, &p.id, new_pin_hash.as_ref().map(|(h, s)| (h.as_str(), s.as_str()))).await?;
    }
    let key = keys::new_key();
    state.db.pocket_key_insert(code, &p.id, &keys::hash_key(&key), now).await?;
    let row = state.db.pocket_player(code, &p.id).await?.ok_or_else(PocketError::internal)?;
    changed(state, code).await?;
    Ok((row, key, notices))
}

fn no_such_name() -> PocketError {
    PocketError::new(StatusCode::NOT_FOUND, "no_such_name", "Nobody joined with that name. Check it's spelled the way you joined.")
}

fn hard_locked() -> PocketError {
    PocketError::new(StatusCode::LOCKED, "hard_locked", "Too many wrong PINs. Ask the host for a reset code.").with(json!({ "tries_left": 0 }))
}

/// The counters after one more wrong try (pure, so the rules are easy to test).
pub fn after_wrong_try(p: &PlayerRow, now: DateTime<Utc>) -> PinCounters {
    let mut c = PinCounters {
        wrong_tries: p.wrong_tries + 1,
        total_wrong: p.total_wrong + 1,
        locked_until: None,
        pin_locked: p.pin_locked,
        last_wrong_at: Some(now),
    };
    if c.total_wrong >= PIN_HARD_LOCK {
        c.pin_locked = true;
        c.wrong_tries = 0;
    } else if c.wrong_tries >= PIN_TRIES {
        c.locked_until = Some(now + Duration::minutes(PIN_WAIT_MINUTES));
        c.wrong_tries = 0;
    }
    c
}

async fn wrong_try(state: &AppState, code: &str, p: &PlayerRow, now: DateTime<Utc>) -> PResult<PocketError> {
    let c = after_wrong_try(p, now);
    state.db.pocket_set_pin_counters(code, &p.id, &c).await?;
    refresh(state, code).await?;
    Ok(if c.pin_locked {
        hard_locked()
    } else if let Some(until) = c.locked_until {
        PocketError::new(StatusCode::LOCKED, "locked", "Too many wrong PINs. Wait 10 minutes, then try again.").with(json!({ "until": until, "tries_left": 0 }))
    } else {
        let left = (PIN_TRIES - c.wrong_tries).min(PIN_HARD_LOCK - c.total_wrong);
        PocketError::new(StatusCode::UNAUTHORIZED, "wrong_pin", format!("That name and PIN don't match. {left} more tries, then a 10-minute wait."))
            .with(json!({ "tries_left": left }))
    })
}

/// The guest has seen their notices (a wrong PIN try): they stop showing.
pub async fn notices_seen(state: &AppState, code: &str, player_id: &str) -> PResult<()> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    state.db.pocket_notices_seen(code, player_id).await?;
    state.db.pocket_bump(code).await?;
    changed(state, code).await
}

/// A named action from a guest or the host, forwarded to the kit.
pub async fn act(state: &AppState, code: &str, actor: &Viewer, name: &str, body: &Value) -> PResult<()> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    let kit = s.kit().ok_or_else(PocketError::not_found)?;
    let rules = effective_rules(kit, &s.spec);
    let allowed = match actor {
        Viewer::Guest(_) => rules.guest_actions,
        Viewer::Host => rules.host_actions,
        Viewer::BigScreen => &[],
    };
    if !allowed.contains(&name) {
        return Err(PocketError::new(StatusCode::BAD_REQUEST, "unknown_action", "That action isn't one this pocket knows.").with(json!({ "actions": allowed })));
    }
    if s.row.state == ENDED {
        return Err(PocketError::new(StatusCode::GONE, "ended", "This pocket app has ended."));
    }
    let now = Utc::now();
    let mut kit_state = s.kit_state.clone();
    let mut ctx = KitCtx::new(&s.spec, &s.players, now);
    kit.act(&mut ctx, &mut kit_state, actor, name, body).map_err(kit_refusal)?;
    save(state, code, &kit_state, ctx, now).await?;
    // pa3b: a guest's quiz answer may be a near miss; the live judge looks at it off this request
    if matches!(actor, Viewer::Guest(_)) && name == "answer" {
        crate::server::quizmaster_live::after_answer(state, code, kit.id());
    }
    // pa13a: a guest's message goes to the bot, a host's reply into the bot's thread
    if kit.id() == desk::ID {
        crate::server::desk_turns::after_act(state, code, actor, name, body).await;
    }
    Ok(())
}

fn kit_refusal(e: KitError) -> PocketError {
    match e {
        KitError::BadInput(m) => PocketError::new(StatusCode::BAD_REQUEST, "bad_action", m),
        KitError::NotNow(m) => PocketError::new(StatusCode::CONFLICT, "not_now", m),
        KitError::Coded { status, code, message, extra } => {
            PocketError::new(StatusCode::from_u16(status).unwrap_or(StatusCode::CONFLICT), code, message).with(extra)
        }
    }
}

/// pa3b: the bot's strict judge settles a near miss (`{q, player, verdict}`) under the pocket's
/// lock, then saves and pushes views like a host action. Server side only: no route calls this.
pub async fn judge(state: &AppState, code: &str, body: &Value) -> PResult<()> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    let kit = s.kit().ok_or_else(PocketError::not_found)?;
    if s.row.state == ENDED {
        return Err(PocketError::new(StatusCode::GONE, "ended", "This pocket app has ended."));
    }
    let now = Utc::now();
    let mut kit_state = s.kit_state.clone();
    let mut ctx = KitCtx::new(&s.spec, &s.players, now);
    kit.judge(&mut ctx, &mut kit_state, body).map_err(kit_refusal)?;
    save(state, code, &kit_state, ctx, now).await
}

/// pa13a: one step of the server's own work on a pocket (the front desk's bot turns), under the
/// pocket's lock, saved and pushed like an action. Server side only: no route calls this.
pub async fn server_event(state: &AppState, code: &str, name: &str, body: &Value) -> PResult<Value> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    let kit = s.kit().ok_or_else(PocketError::not_found)?;
    if s.row.state == ENDED {
        return Err(PocketError::new(StatusCode::GONE, "ended", "This pocket app has ended."));
    }
    let now = Utc::now();
    let mut kit_state = s.kit_state.clone();
    let mut ctx = KitCtx::new(&s.spec, &s.players, now);
    let out = kit.server_event(&mut ctx, &mut kit_state, name, body).map_err(kit_refusal)?;
    save(state, code, &kit_state, ctx, now).await?;
    Ok(out)
}

/// The host removes a player: their keys stop working and the kit forgets them.
pub async fn remove_player(state: &AppState, code: &str, player_id: &str) -> PResult<()> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    let kit = s.kit().ok_or_else(PocketError::not_found)?;
    let now = Utc::now();
    if s.players.iter().any(|p| p.id == player_id) {
        kit.may_remove(&KitCtx::new(&s.spec, &s.players, now), &s.kit_state, player_id).map_err(kit_refusal)?;
    }
    if !state.db.pocket_player_remove(code, player_id, now).await? {
        return Err(PocketError::new(StatusCode::NOT_FOUND, "no_such_player", "That player isn't in this pocket."));
    }
    state.pocket.signal(code, Signal::Revoked(player_id.to_string()));
    let players: Vec<Player> = s.players.iter().filter(|p| p.id != player_id).cloned().collect();
    let mut kit_state = s.kit_state.clone();
    let mut ctx = KitCtx::new(&s.spec, &players, now);
    kit.on_remove(&mut ctx, &mut kit_state, player_id);
    let gone = threads_gone(kit, &s.kit_state, &kit_state);
    save(state, code, &kit_state, ctx, now).await?;
    drop_threads(state, &gone).await;
    Ok(())
}

/// pa13a: forgets the players whose data has expired (a front desk guest 7 days after their last
/// message): their row, keys, events, the bot's conversation with them and their kit state.
/// Returns how many went.
pub async fn forget_idle(state: &AppState, code: &str, now: DateTime<Utc>) -> PResult<usize> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let Some(s) = snapshot(state, code).await? else { return Ok(0) };
    let Some(kit) = s.kit() else { return Ok(0) };
    let idle = kit.idle_players(&KitCtx::new(&s.spec, &s.players, now), &s.kit_state);
    if idle.is_empty() {
        return Ok(0);
    }
    let players: Vec<Player> = s.players.iter().filter(|p| !idle.contains(&p.id)).cloned().collect();
    let mut kit_state = s.kit_state.clone();
    let mut ctx = KitCtx::new(&s.spec, &players, now);
    for id in &idle {
        state.db.pocket_player_forget(code, id).await?;
        state.pocket.signal(code, Signal::Revoked(id.clone()));
        kit.on_remove(&mut ctx, &mut kit_state, id);
    }
    let gone = threads_gone(kit, &s.kit_state, &kit_state);
    save(state, code, &kit_state, ctx, now).await?;
    drop_threads(state, &gone).await;
    Ok(idle.len())
}

/// The host's PIN reset: a one-time 4-character code, valid 24 h. The old PIN and every guest key
/// of the player stop working. Returns the code (shown once) and when it expires; nothing else.
pub async fn reset_pin(state: &AppState, code: &str, player_id: &str) -> PResult<(String, DateTime<Utc>)> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    if !s.rules().is_some_and(|r| r.needs_pin) {
        return Err(PocketError::new(StatusCode::CONFLICT, "no_pins", "This pocket has no PINs."));
    }
    let p = state.db.pocket_player(code, player_id).await?.filter(|p| p.removed_at.is_none());
    let p = p.ok_or_else(|| PocketError::new(StatusCode::NOT_FOUND, "no_such_player", "That player isn't in this pocket."))?;
    let salt = match p.pin_salt {
        Some(s) => s,
        None => {
            let s = keys::new_salt();
            state.db.pocket_set_salt(code, player_id, &s).await?;
            s
        }
    };
    let now = Utc::now();
    let reset = keys::new_reset_code();
    let expires = now + Duration::hours(RESET_CODE_HOURS);
    state.db.pocket_pin_reset(code, player_id, &keys::hash_secret(&reset, &salt), expires, now).await?;
    state.pocket.signal(code, Signal::Revoked(player_id.to_string()));
    state.db.pocket_bump(code).await?;
    changed(state, code).await?;
    Ok((reset, expires))
}

/// Lets each watched pocket's kit run its timers.
pub async fn tick(state: &AppState) {
    for code in state.pocket.watched() {
        tick_one(state, &code, Utc::now(), false).await;
    }
}

/// Runs the timers of open pockets whose kit asks for it even while nobody watches (a Secret
/// Santa draw or reveal weeks away). The sweeper calls it once a minute, with its clock.
pub async fn tick_unwatched(state: &AppState, now: DateTime<Utc>) {
    for code in state.db.pocket_codes_open().await.unwrap_or_default() {
        tick_one(state, &code, now, true).await;
    }
}

async fn tick_one(state: &AppState, code: &str, now: DateTime<Utc>, only_unwatched_kits: bool) {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let Ok(Some(s)) = snapshot(state, code).await else { return };
    let Some(kit) = s.kit() else { return };
    if s.row.state != OPEN || (only_unwatched_kits && !kit.ticks_unwatched()) {
        return;
    }
    let mut kit_state = s.kit_state.clone();
    let mut ctx = KitCtx::new(&s.spec, &s.players, now);
    if kit.tick(&mut ctx, &mut kit_state) {
        if let Err(e) = save(state, code, &kit_state, ctx, now).await {
            tracing::warn!(code = %code, "pocket apps: couldn't save a timer change: {}", e.code);
        }
    }
}

/// What one sweep did.
#[derive(Debug, Default, PartialEq)]
pub struct Swept {
    pub ended: usize,
    pub deleted: usize,
}

/// Runs unwatched kit timers, ends pockets past their closing time and deletes pockets whose
/// data has expired. Does nothing while the switch is off.
pub async fn sweep(state: &AppState, now: DateTime<Utc>) -> Swept {
    let mut out = Swept::default();
    if !enabled(&state.db).await {
        return out;
    }
    tick_unwatched(state, now).await;
    // pa13a: front desk guests whose data has expired, then turns a restart left queued
    for code in state.db.pocket_codes_of_kit(desk::ID).await.unwrap_or_default() {
        match forget_idle(state, &code, now).await {
            Ok(n) if n > 0 => tracing::info!(count = n, "pocket apps: deleted expired guests' data"),
            Ok(_) => {}
            Err(e) => tracing::warn!(code = %code, "pocket apps: couldn't delete expired guests: {}", e.code),
        }
    }
    crate::server::desk_turns::resume(state).await;
    for code in state.db.pocket_past_closing(now).await.unwrap_or_default() {
        if end(state, &code).await.is_ok() {
            out.ended += 1;
        }
    }
    match state.db.pocket_expired(now).await {
        Ok(codes) => {
            for code in codes {
                match delete(state, &code).await {
                    Ok(true) => out.deleted += 1,
                    Ok(false) => {}
                    Err(e) => tracing::warn!(code = %code, "pocket apps: couldn't delete an expired pocket: {}", e.code),
                }
            }
        }
        Err(e) => tracing::warn!("pocket apps: couldn't list expired pockets: {e}"),
    }
    if out.deleted > 0 {
        tracing::info!(count = out.deleted, "pocket apps: deleted expired pockets and their guests' data");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player() -> PlayerRow {
        PlayerRow {
            pocket_code: "abcdef".into(),
            id: "pl_1".into(),
            name: "Mia".into(),
            name_key: "mia".into(),
            avatar: String::new(),
            team: None,
            pin_hash: Some("x".into()),
            pin_salt: Some("y".into()),
            wrong_tries: 0,
            total_wrong: 0,
            locked_until: None,
            pin_locked: false,
            last_wrong_at: None,
            reset_code_hash: None,
            reset_expires_at: None,
            pin_reset_at: None,
            joined_at: Utc::now(),
            removed_at: None,
        }
    }

    #[test]
    fn five_wrong_then_a_wait_then_ten_in_total_locks() {
        let now = Utc::now();
        let mut p = player();
        for i in 1..=4 {
            let c = after_wrong_try(&p, now);
            assert_eq!((c.wrong_tries, c.total_wrong, c.locked_until, c.pin_locked), (i, i, None, false));
            p.wrong_tries = c.wrong_tries;
            p.total_wrong = c.total_wrong;
        }
        let c = after_wrong_try(&p, now);
        assert_eq!(c.locked_until, Some(now + Duration::minutes(10)), "the 5th wrong try waits 10 minutes");
        assert!(!c.pin_locked);
        p.wrong_tries = c.wrong_tries;
        p.total_wrong = c.total_wrong;
        for _ in 6..=9 {
            let c = after_wrong_try(&p, now);
            assert!(!c.pin_locked && c.locked_until.is_none());
            p.wrong_tries = c.wrong_tries;
            p.total_wrong = c.total_wrong;
        }
        let c = after_wrong_try(&p, now);
        assert!(c.pin_locked, "the 10th wrong try in total locks the name until a reset");
    }
}
